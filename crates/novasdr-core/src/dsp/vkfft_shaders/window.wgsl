struct Params {
  len: u32,
  base_idx: u32,
  src_offset: u32,
  dst_offset: u32,
  power_offset: i32,
  _pad0: i32,
  normalize: f32,
  _pad1: f32,
}

struct ComplexBuf {
  data: array<vec2<f32>>,
}

struct WindowBuf {
  data: array<f32>,
}

@group(0) @binding(0) var<storage, read_write> complexbuf: ComplexBuf;
@group(0) @binding(1) var<storage, read> windowbuf: WindowBuf;

var<push_constant> pc: Params;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let i = gid.x;
  if (i >= pc.len) {
    return;
  }

  let w = windowbuf.data[i];
  complexbuf.data[i] = complexbuf.data[i] * vec2<f32>(w, w);
}
