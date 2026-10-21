/**
 * Paints decoded frames on the viewer canvas.
 *
 * With WebGL 2 the canvas is sized to the device pixels it covers and the picture is
 * scaled by a shader: Catmull-Rom bicubic when enlarging (sharper text than the browser's
 * bilinear stretch), nearest neighbour for whole multiples and 1:1, where every remote pixel
 * becomes an exact block of screen pixels. Smaller than the frame, the canvas keeps the
 * frame's size and the browser shrinks it, as before. Without WebGL 2 the frame is drawn
 * with the 2D context at its own size and CSS scales it.
 */

/** How the picture is enlarged on screen. */
export type Filter = "bicubic" | "nearest";

const VERTEX = `#version 300 es
out vec2 uv;
void main() {
  // One triangle covering the viewport; uv (0,0) is the top left corner of the frame.
  vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
  uv = vec2(p.x, 1.0 - p.y);
  gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}`;

const FRAGMENT = `#version 300 es
precision highp float;
uniform sampler2D frame;
uniform bool bicubic;
in vec2 uv;
out vec4 color;

vec4 texel(ivec2 p, ivec2 size) {
  return texelFetch(frame, clamp(p, ivec2(0), size - 1), 0);
}

vec4 catmullRom(float t) {
  float t2 = t * t;
  float t3 = t2 * t;
  return vec4(-0.5 * t3 + t2 - 0.5 * t,
              1.5 * t3 - 2.5 * t2 + 1.0,
              -1.5 * t3 + 2.0 * t2 + 0.5 * t,
              0.5 * t3 - 0.5 * t2);
}

void main() {
  ivec2 size = textureSize(frame, 0);
  vec2 p = uv * vec2(size);
  if (!bicubic) {
    color = texel(ivec2(floor(p)), size);
    return;
  }
  p -= 0.5;
  vec2 f = floor(p);
  vec4 wx = catmullRom(p.x - f.x);
  vec4 wy = catmullRom(p.y - f.y);
  ivec2 base = ivec2(f) - 1;
  vec3 sum = vec3(0.0);
  for (int j = 0; j < 4; j++) {
    vec3 row = vec3(0.0);
    for (int i = 0; i < 4; i++) row += wx[i] * texel(base + ivec2(i, j), size).rgb;
    sum += wy[j] * row;
  }
  color = vec4(clamp(sum, 0.0, 1.0), 1.0);
}`;

interface Gl {
  gl: WebGL2RenderingContext;
  bicubic: WebGLUniformLocation | null;
}

function compile(gl: WebGL2RenderingContext, type: number, source: string): WebGLShader {
  const shader = gl.createShader(type)!;
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    throw new Error(gl.getShaderInfoLog(shader) ?? "shader");
  }
  return shader;
}

function setUp(gl: WebGL2RenderingContext): Gl {
  const program = gl.createProgram()!;
  gl.attachShader(program, compile(gl, gl.VERTEX_SHADER, VERTEX));
  gl.attachShader(program, compile(gl, gl.FRAGMENT_SHADER, FRAGMENT));
  gl.linkProgram(program);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
    throw new Error(gl.getProgramInfoLog(program) ?? "program");
  }
  gl.useProgram(program);
  gl.bindVertexArray(gl.createVertexArray());
  gl.bindTexture(gl.TEXTURE_2D, gl.createTexture());
  for (const [name, value] of [
    [gl.TEXTURE_MIN_FILTER, gl.NEAREST],
    [gl.TEXTURE_MAG_FILTER, gl.NEAREST],
    [gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE],
    [gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE],
  ]) {
    gl.texParameteri(gl.TEXTURE_2D, name, value);
  }
  gl.uniform1i(gl.getUniformLocation(program, "frame"), 0);
  return { gl, bicubic: gl.getUniformLocation(program, "bicubic") };
}

const GL_OPTIONS: WebGLContextAttributes = {
  alpha: false,
  antialias: false,
  depth: false,
  desynchronized: true,
  powerPreference: "low-power",
};

let webgl2Checked: boolean | undefined;

function webgl2Works(): boolean {
  if (webgl2Checked === undefined) {
    try {
      const gl = document.createElement("canvas").getContext("webgl2", GL_OPTIONS);
      webgl2Checked = !!gl && !!setUp(gl);
      gl?.getExtension("WEBGL_lose_context")?.loseContext();
    } catch {
      webgl2Checked = false;
    }
  }
  return webgl2Checked;
}

export class VideoRenderer {
  private webgl: Gl | null = null;
  private ctx: CanvasRenderingContext2D | null = null;
  /** Where frames go when WebGL cannot take a `VideoFrame` directly. */
  private staging: CanvasRenderingContext2D | null = null;
  private output: { width: number; height: number; filter: Filter } | null = null;
  private hasFrame = false;
  /** Size of the last frame painted (0 before the first one). */
  width = 0;
  height = 0;

  constructor(private canvas: HTMLCanvasElement) {
    // A canvas keeps the first kind of context it gives: try WebGL on a spare one first.
    const gl = webgl2Works() ? canvas.getContext("webgl2", GL_OPTIONS) : null;
    if (gl) {
      this.webgl = setUp(gl);
      canvas.addEventListener("webglcontextlost", (e) => {
        e.preventDefault();
        this.hasFrame = false;
      });
      canvas.addEventListener("webglcontextrestored", () => {
        this.webgl = setUp(gl);
      });
    } else {
      const ctx = canvas.getContext("2d", { alpha: false, desynchronized: true });
      if (!ctx) throw new Error("canvas non disponibile");
      this.ctx = ctx;
    }
  }

  /** Whether the shader scaling is available (otherwise CSS scales the picture). */
  get scales(): boolean {
    return this.webgl !== null;
  }

  /** Size in device pixels the picture covers on screen, and how to enlarge it. */
  setOutput(width: number, height: number, filter: Filter) {
    this.output = { width: Math.round(width), height: Math.round(height), filter };
    if (this.ctx) {
      this.canvas.style.imageRendering = filter === "nearest" ? "pixelated" : "";
    } else if (this.hasFrame) {
      this.draw();
    }
  }

  /** Paints a frame; returns true when its size differs from the previous one. */
  paint(frame: VideoFrame): boolean {
    const { displayWidth: w, displayHeight: h } = frame;
    const resized = w !== this.width || h !== this.height;
    this.width = w;
    this.height = h;
    if (this.ctx) {
      if (this.canvas.width !== w || this.canvas.height !== h) {
        this.canvas.width = w;
        this.canvas.height = h;
      }
      this.ctx.drawImage(frame, 0, 0);
      return resized;
    }
    const { gl } = this.webgl!;
    try {
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, this.source(frame));
    } catch {
      // Some WebViews cannot upload a VideoFrame: go through a 2D canvas from now on.
      this.staging ??= document.createElement("canvas").getContext("2d");
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, this.source(frame));
    }
    this.hasFrame = true;
    this.draw();
    return resized;
  }

  private source(frame: VideoFrame): TexImageSource {
    const staging = this.staging;
    if (!staging) return frame;
    if (staging.canvas.width !== this.width || staging.canvas.height !== this.height) {
      staging.canvas.width = this.width;
      staging.canvas.height = this.height;
    }
    staging.drawImage(frame, 0, 0);
    return staging.canvas;
  }

  private draw() {
    const { gl, bicubic } = this.webgl!;
    if (gl.isContextLost()) return;
    const out = this.output;
    // Not larger than the frame (or no layout yet): the frame's own pixels, shrunk by CSS.
    const enlarge = !!out && out.width > this.width && out.height > this.height;
    const width = enlarge ? out.width : this.width;
    const height = enlarge ? out.height : this.height;
    if (this.canvas.width !== width || this.canvas.height !== height) {
      this.canvas.width = width;
      this.canvas.height = height;
    }
    gl.viewport(0, 0, width, height);
    gl.uniform1i(bicubic, enlarge && out.filter === "bicubic" ? 1 : 0);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
  }
}
