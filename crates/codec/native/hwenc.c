// Thin C layer over FFmpeg's hardware H.264 encoder (ADR 0058), so the Rust side needs no
// bindings to FFmpeg's structs: open, change the rate, encode one image, close.
//
// macOS: images are CVPixelBuffers (zero copy from ScreenCaptureKit, or wrapping a BGRA
// buffer); FFmpeg hands them to VideoToolbox as they are.
// Windows: BGRA is converted to NV12 (swscale, SIMD) for the Media Foundation hardware
// encoder (Intel, AMD, NVIDIA), in low latency "display remoting" mode.

#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include <libavcodec/avcodec.h>
#include <libavutil/error.h>
#include <libavutil/frame.h>
#include <libavutil/opt.h>

#ifdef __APPLE__
#include <CoreVideo/CoreVideo.h>
#endif
#ifdef _WIN32
#include <windows.h>
#include <mmsystem.h>
#include <libswscale/swscale.h>

/// Longest wait for the packet of the frame just sent (the encoder works asynchronously).
#define PACKET_WAIT_MS 60
#endif

typedef struct KrHwEncoder {
    AVCodecContext *ctx;
    AVPacket *pkt;
    int fps;
#ifdef __APPLE__
    /// IOSurface-backed buffers for BGRA images (scaled frames), made on first use.
    CVPixelBufferPoolRef pool;
#endif
#ifdef _WIN32
    struct SwsContext *sws;
    AVFrame *nv12;
    int timer_period;
#endif
} KrHwEncoder;

static void describe(int err, char *out, int len) {
    if (out && len > 0) av_strerror(err, out, (size_t)len);
}

void kr_hw_close(KrHwEncoder *enc) {
    if (!enc) return;
    avcodec_free_context(&enc->ctx);
    av_packet_free(&enc->pkt);
#ifdef __APPLE__
    if (enc->pool) CVPixelBufferPoolRelease(enc->pool);
#endif
#ifdef _WIN32
    sws_freeContext(enc->sws);
    av_frame_free(&enc->nv12);
    if (enc->timer_period) timeEndPeriod(1);
#endif
    av_free(enc);
}

/// Opens the platform's hardware encoder at `width`x`height`, or NULL with a reason in
/// `err`. `gop` is the keyframe interval in frames.
KrHwEncoder *kr_hw_open(int width, int height, int fps, int64_t bit_rate, int gop, char *err,
                        int err_len) {
#if defined(__APPLE__)
    const AVCodec *codec = avcodec_find_encoder_by_name("h264_videotoolbox");
#elif defined(_WIN32)
    const AVCodec *codec = avcodec_find_encoder_by_name("h264_mf");
#else
    const AVCodec *codec = NULL;
#endif
    if (!codec) {
        snprintf(err, (size_t)err_len, "hardware encoder not available");
        return NULL;
    }
    KrHwEncoder *enc = av_mallocz(sizeof(*enc));
    if (!enc) return NULL;
    enc->ctx = avcodec_alloc_context3(codec);
    enc->pkt = av_packet_alloc();
    if (!enc->ctx || !enc->pkt) {
        kr_hw_close(enc);
        snprintf(err, (size_t)err_len, "out of memory");
        return NULL;
    }
    AVCodecContext *ctx = enc->ctx;
    enc->fps = fps > 0 ? fps : 30;
    ctx->width = width;
    ctx->height = height;
    ctx->time_base = (AVRational){1, 1000};
    ctx->framerate = (AVRational){enc->fps, 1};
    ctx->bit_rate = bit_rate;
    // A single frame may overshoot a little, the second as a whole may not.
    ctx->rc_max_rate = bit_rate + bit_rate / 2;
    ctx->gop_size = gop > 0 ? gop : 1800;
    ctx->max_b_frames = 0;
    ctx->flags |= AV_CODEC_FLAG_LOW_DELAY;
#ifdef __APPLE__
    ctx->pix_fmt = AV_PIX_FMT_VIDEOTOOLBOX;
    ctx->sw_pix_fmt = AV_PIX_FMT_BGRA;
    av_opt_set_int(ctx->priv_data, "realtime", 1, 0);
    av_opt_set_int(ctx->priv_data, "allow_sw", 0, 0);
    av_opt_set(ctx->priv_data, "profile", "high", 0);
#endif
#ifdef _WIN32
    ctx->pix_fmt = AV_PIX_FMT_NV12;
    ctx->profile = AV_PROFILE_H264_HIGH;
    av_opt_set_int(ctx->priv_data, "hw_encoding", 1, 0);
    av_opt_set(ctx->priv_data, "scenario", "display_remoting", 0);
    av_opt_set(ctx->priv_data, "rate_control", "pc_vbr", 0);
#endif
    int ret = avcodec_open2(ctx, codec, NULL);
    if (ret < 0) {
        describe(ret, err, err_len);
        kr_hw_close(enc);
        return NULL;
    }
#ifdef _WIN32
    enc->sws = sws_getContext(width, height, AV_PIX_FMT_BGRA, width, height, AV_PIX_FMT_NV12,
                              SWS_POINT, NULL, NULL, NULL);
    enc->nv12 = av_frame_alloc();
    if (enc->nv12) {
        enc->nv12->format = AV_PIX_FMT_NV12;
        enc->nv12->width = width;
        enc->nv12->height = height;
    }
    if (!enc->sws || !enc->nv12 || av_frame_get_buffer(enc->nv12, 0) < 0) {
        snprintf(err, (size_t)err_len, "cannot set up the colour conversion");
        kr_hw_close(enc);
        return NULL;
    }
    // Millisecond waits for the packet (the default timer ticks every ~16 ms).
    enc->timer_period = timeBeginPeriod(1) == TIMERR_NOERROR;
#endif
    return enc;
}

/// New target bitrate and frame rate, applied from the next frame.
void kr_hw_set_rate(KrHwEncoder *enc, int64_t bit_rate, int fps) {
    if (!enc || bit_rate <= 0) return;
    enc->ctx->bit_rate = bit_rate;
    enc->ctx->rc_max_rate = bit_rate + bit_rate / 2;
    if (fps > 0) enc->fps = fps;
}

/// After a successful encode with a packet: the access unit (Annex B), valid until the next
/// call on this encoder.
typedef struct KrPacket {
    const uint8_t *data;
    int size;
    int keyframe;
} KrPacket;

#ifdef __APPLE__
static void release_pixel_buffer(void *opaque, uint8_t *data) {
    (void)data;
    CVPixelBufferRelease((CVPixelBufferRef)opaque);
}

/// Encodes one CVPixelBuffer (retained here for as long as the encoder needs it).
/// Returns 1 with `out` filled, 0 when the frame gave no packet, or a negative error.
static int encode_pixel_buffer(KrHwEncoder *enc, CVPixelBufferRef image, int64_t pts_ms,
                               int keyframe, KrPacket *out, char *err, int err_len) {
    AVFrame *frame = av_frame_alloc();
    if (!frame) return AVERROR(ENOMEM);
    CVPixelBufferRetain(image);
    frame->buf[0] = av_buffer_create((uint8_t *)image, 1, release_pixel_buffer, image, 0);
    if (!frame->buf[0]) {
        CVPixelBufferRelease(image);
        av_frame_free(&frame);
        return AVERROR(ENOMEM);
    }
    frame->format = AV_PIX_FMT_VIDEOTOOLBOX;
    frame->width = enc->ctx->width;
    frame->height = enc->ctx->height;
    frame->data[3] = (uint8_t *)image;
    frame->pts = pts_ms;
    frame->pict_type = keyframe ? AV_PICTURE_TYPE_I : AV_PICTURE_TYPE_NONE;

    av_packet_unref(enc->pkt);
    int ret = avcodec_send_frame(enc->ctx, frame);
    av_frame_free(&frame);
    if (ret < 0) {
        describe(ret, err, err_len);
        return ret;
    }
    ret = avcodec_receive_packet(enc->ctx, enc->pkt);
    if (ret == AVERROR(EAGAIN)) return 0;
    if (ret < 0) {
        describe(ret, err, err_len);
        return ret;
    }
    out->data = enc->pkt->data;
    out->size = enc->pkt->size;
    out->keyframe = (enc->pkt->flags & AV_PKT_FLAG_KEY) != 0;
    return 1;
}

/// Encodes a CVPixelBufferRef as captured (zero copy).
int kr_hw_encode_native(KrHwEncoder *enc, void *image, int64_t pts_ms, int keyframe,
                        KrPacket *out, char *err, int err_len) {
    return encode_pixel_buffer(enc, (CVPixelBufferRef)image, pts_ms, keyframe, out, err,
                               err_len);
}
#endif

/// Encodes a tightly packed BGRA image of the encoder's size.
int kr_hw_encode_bgra(KrHwEncoder *enc, const uint8_t *bgra, int64_t pts_ms, int keyframe,
                      KrPacket *out, char *err, int err_len) {
#if defined(_WIN32)
    AVFrame *frame = enc->nv12;
    int ret = av_frame_make_writable(frame);
    if (ret < 0) {
        describe(ret, err, err_len);
        return ret;
    }
    const uint8_t *src[1] = {bgra};
    const int src_stride[1] = {enc->ctx->width * 4};
    sws_scale(enc->sws, src, src_stride, 0, enc->ctx->height, frame->data, frame->linesize);
    frame->pts = pts_ms;
    frame->pict_type = keyframe ? AV_PICTURE_TYPE_I : AV_PICTURE_TYPE_NONE;

    av_packet_unref(enc->pkt);
    ret = avcodec_send_frame(enc->ctx, frame);
    if (ret < 0 && ret != AVERROR(EAGAIN)) {
        describe(ret, err, err_len);
        return ret;
    }
    // The hardware encoder answers asynchronously: wait briefly for this frame's packet.
    for (int waited = 0;; waited++) {
        ret = avcodec_receive_packet(enc->ctx, enc->pkt);
        if (ret >= 0) break;
        if (ret != AVERROR(EAGAIN)) {
            describe(ret, err, err_len);
            return ret;
        }
        if (waited >= PACKET_WAIT_MS) return 0;
        Sleep(1);
    }
    out->data = enc->pkt->data;
    out->size = enc->pkt->size;
    out->keyframe = (enc->pkt->flags & AV_PKT_FLAG_KEY) != 0;
    return 1;
#elif defined(__APPLE__)
    CVPixelBufferRef image = NULL;
    size_t width = (size_t)enc->ctx->width, height = (size_t)enc->ctx->height;
    if (!enc->pool) {
        int32_t w = (int32_t)width, h = (int32_t)height, format = kCVPixelFormatType_32BGRA;
        CFNumberRef nw = CFNumberCreate(NULL, kCFNumberSInt32Type, &w);
        CFNumberRef nh = CFNumberCreate(NULL, kCFNumberSInt32Type, &h);
        CFNumberRef nf = CFNumberCreate(NULL, kCFNumberSInt32Type, &format);
        CFDictionaryRef surface = CFDictionaryCreate(NULL, NULL, NULL, 0,
                                                     &kCFTypeDictionaryKeyCallBacks,
                                                     &kCFTypeDictionaryValueCallBacks);
        const void *keys[] = {kCVPixelBufferWidthKey, kCVPixelBufferHeightKey,
                              kCVPixelBufferPixelFormatTypeKey,
                              kCVPixelBufferIOSurfacePropertiesKey};
        const void *values[] = {nw, nh, nf, surface};
        CFDictionaryRef attributes = CFDictionaryCreate(NULL, keys, values, 4,
                                                        &kCFTypeDictionaryKeyCallBacks,
                                                        &kCFTypeDictionaryValueCallBacks);
        CVPixelBufferPoolCreate(NULL, NULL, attributes, &enc->pool);
        CFRelease(attributes);
        CFRelease(surface);
        CFRelease(nf);
        CFRelease(nh);
        CFRelease(nw);
    }
    CVReturn status = enc->pool
        ? CVPixelBufferPoolCreatePixelBuffer(NULL, enc->pool, &image)
        : kCVReturnError;
    if (status != kCVReturnSuccess) {
        snprintf(err, (size_t)err_len, "no pixel buffer: %d", (int)status);
        return AVERROR_EXTERNAL;
    }
    CVPixelBufferLockBaseAddress(image, 0);
    uint8_t *dst = CVPixelBufferGetBaseAddress(image);
    size_t stride = CVPixelBufferGetBytesPerRow(image);
    for (size_t y = 0; y < height; y++)
        memcpy(dst + y * stride, bgra + y * width * 4, width * 4);
    CVPixelBufferUnlockBaseAddress(image, 0);
    int ret = encode_pixel_buffer(enc, image, pts_ms, keyframe, out, err, err_len);
    CVPixelBufferRelease(image);
    return ret;
#else
    (void)enc; (void)bgra; (void)pts_ms; (void)keyframe; (void)out;
    snprintf(err, (size_t)err_len, "not supported");
    return AVERROR(ENOSYS);
#endif
}
