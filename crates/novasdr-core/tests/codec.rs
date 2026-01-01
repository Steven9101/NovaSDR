use novasdr_core::codec::{flac_stream::FlacStreamEncoder, zstd_stream::ZstdStreamEncoder};
use zstd_safe::{DCtx, InBuffer, OutBuffer};

#[test]
fn flac_header_starts_with_magic() {
    let enc = FlacStreamEncoder::new(12_000, 8, 512).unwrap();
    let header = enc.header_bytes().unwrap();
    assert!(header.starts_with(b"fLaC"));
}

#[test]
fn zstd_stream_flush_roundtrip() {
    let mut enc = ZstdStreamEncoder::new(3).unwrap();
    let input = b"hello zstd stream";
    let out = enc.compress_flush(input).unwrap();

    let mut dctx = DCtx::create();
    let mut dst = vec![0u8; 1024];
    let pos = {
        let mut out_buf = OutBuffer::around(&mut dst[..]);
        let mut in_buf = InBuffer::around(&out);
        while in_buf.pos < in_buf.src.len() && out_buf.pos() < out_buf.capacity() {
            let _ = dctx.decompress_stream(&mut out_buf, &mut in_buf).unwrap();
        }
        out_buf.pos()
    };
    dst.truncate(pos);
    assert_eq!(&dst, input);
}
