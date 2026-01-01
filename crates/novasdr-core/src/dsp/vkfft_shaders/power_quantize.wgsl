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

struct PowerBuf {
  data: array<f32>,
}

struct QuantBuf {
  data: array<i32>,
}

@group(0) @binding(0) var<storage, read_write> complexbuf: ComplexBuf;
@group(0) @binding(2) var<storage, read_write> powerbuf: PowerBuf;
@group(0) @binding(3) var<storage, read_write> quantbuf: QuantBuf;

var<push_constant> pc: Params;

fn quantize_power(power: f32, power_offset: i32) -> i32 {
  let p = max(power, 1e-30);
  let db = log(p) * 8.685889638 + 127.0 + f32(power_offset) * 6.020599913279624;
  let clamped = clamp(round(db), -128.0, 127.0);
  return i32(clamped);
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let i = gid.x;
  if (i >= pc.len) {
    return;
  }

  var src = i + pc.base_idx;
  if (src >= pc.len) {
    src = src - pc.len;
  }

  let v = complexbuf.data[src] / vec2<f32>(pc.normalize, pc.normalize);
  let p = max(v.x * v.x + v.y * v.y, 0.0);

  let out_idx = pc.dst_offset + i;
  powerbuf.data[out_idx] = p;
  quantbuf.data[out_idx] = quantize_power(p, pc.power_offset);
}
