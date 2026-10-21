import { VideoRenderer } from "./video-renderer";

/**
 * Decodes the partner's H.264 stream with WebCodecs (hardware accelerated where available)
 * and paints it on a canvas (`VideoRenderer`).
 *
 * Wire format of each message from the backend:
 *   [keyframe: u8][timestamp µs: u64 little endian][Annex B access unit]
 */

/** Frames waiting in the decoder before we give up on catching up. */
const MAX_DECODE_QUEUE = 6;

export function webCodecsSupported(): boolean {
  return typeof VideoDecoder !== "undefined" && typeof EncodedVideoChunk !== "undefined";
}

/** Byte offsets of NAL unit payloads (after each start code) in an Annex B buffer. */
function nalUnits(data: Uint8Array): Uint8Array[] {
  const starts: number[] = [];
  for (let i = 0; i + 3 < data.length; i++) {
    if (data[i] === 0 && data[i + 1] === 0 && data[i + 2] === 1) {
      starts.push(i + 3);
      i += 2;
    }
  }
  return starts.map((start, n) => {
    let end = n + 1 < starts.length ? starts[n + 1] - 3 : data.length;
    // A 4-byte start code leaves one extra zero before the next unit.
    if (end > start && data[end - 1] === 0) end--;
    return data.subarray(start, end);
  });
}

/** RFC 6381 codec string ("avc1.42c01f") from the SPS in a keyframe. */
function codecFromKeyframe(data: Uint8Array): string | null {
  const sps = nalUnits(data).find((nal) => (nal[0] & 0x1f) === 7);
  if (!sps || sps.length < 4) return null;
  const hex = (b: number) => b.toString(16).padStart(2, "0");
  return `avc1.${hex(sps[1])}${hex(sps[2])}${hex(sps[3])}`;
}

export class RemoteVideo {
  private decoder: VideoDecoder | null = null;
  readonly renderer: VideoRenderer;
  private frames = 0;
  private fpsWindowStart = performance.now();
  private closed = false;
  private painted = false;

  constructor(
    canvas: HTMLCanvasElement,
    private needKeyframe: () => void,
    private onFps: (fps: number) => void,
    private onFirstFrame: () => void,
    private onSize: (width: number, height: number) => void,
  ) {
    this.renderer = new VideoRenderer(canvas);
  }

  push(message: ArrayBuffer) {
    if (this.closed || message.byteLength < 10) return;
    const view = new DataView(message);
    const keyframe = view.getUint8(0) === 1;
    const timestamp = Number(view.getBigUint64(1, true));
    const data = new Uint8Array(message, 9);

    if (!this.decoder) {
      // Decoding can only start from a keyframe carrying SPS/PPS.
      if (!keyframe) {
        this.needKeyframe();
        return;
      }
      const codec = codecFromKeyframe(data);
      if (!codec) return;
      this.decoder = this.createDecoder(codec);
    }
    // Falling behind (slow decode or a burst after a stall): drop the backlog and
    // resync from the next keyframe rather than showing ever older pictures.
    if (!keyframe && this.decoder.decodeQueueSize > MAX_DECODE_QUEUE) {
      this.reset();
      return;
    }
    try {
      this.decoder.decode(
        new EncodedVideoChunk({ type: keyframe ? "key" : "delta", timestamp, data }),
      );
    } catch {
      this.reset();
    }
  }

  private createDecoder(codec: string): VideoDecoder {
    const decoder = new VideoDecoder({
      output: (frame) => this.paint(frame),
      error: () => this.reset(),
    });
    decoder.configure({ codec, optimizeForLatency: true });
    return decoder;
  }

  private paint(frame: VideoFrame) {
    if (this.closed) {
      frame.close();
      return;
    }
    const resized = this.renderer.paint(frame);
    frame.close();
    if (resized) this.onSize(this.renderer.width, this.renderer.height);

    if (!this.painted) {
      this.painted = true;
      this.onFirstFrame();
    }
    this.frames++;
    const now = performance.now();
    if (now - this.fpsWindowStart >= 1000) {
      this.onFps((this.frames * 1000) / (now - this.fpsWindowStart));
      this.frames = 0;
      this.fpsWindowStart = now;
    }
  }

  /** The stream changed (e.g. another monitor): decode again from the next keyframe. */
  restart() {
    this.reset();
  }

  /** Drops the decoder after an error; the next keyframe restarts it. */
  private reset() {
    try {
      this.decoder?.close();
    } catch {
      // already closed
    }
    this.decoder = null;
    if (!this.closed) this.needKeyframe();
  }

  close() {
    this.closed = true;
    try {
      this.decoder?.close();
    } catch {
      // already closed
    }
    this.decoder = null;
  }
}
