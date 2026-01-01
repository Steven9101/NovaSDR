use zstd_safe::{CCtx, InBuffer, OutBuffer};

pub struct ZstdStreamEncoder {
    cctx: CCtx<'static>,
    level: i32,
}

impl ZstdStreamEncoder {
    pub fn new(level: i32) -> anyhow::Result<Self> {
        let mut cctx = CCtx::create();
        map_zstd(
            cctx.set_parameter(zstd_safe::CParameter::CompressionLevel(level)),
            "set zstd compression level",
        )?;
        Ok(Self { cctx, level })
    }

    pub fn compress_flush(&mut self, input: &[u8]) -> anyhow::Result<Vec<u8>> {
        let max = zstd_safe::compress_bound(input.len());
        let mut out = vec![0u8; max.max(64)];

        let mut in_buf = InBuffer::around(input);
        let pos = {
            let mut out_buf = OutBuffer::around(&mut out[..]);
            map_zstd(
                self.cctx.compress_stream2(
                    &mut out_buf,
                    &mut in_buf,
                    zstd_safe::zstd_sys::ZSTD_EndDirective::ZSTD_e_flush,
                ),
                "zstd compress_stream2 flush",
            )?;
            out_buf.pos()
        };
        out.truncate(pos);
        Ok(out)
    }

    pub fn level(&self) -> i32 {
        self.level
    }
}

fn map_zstd(res: zstd_safe::SafeResult, ctx: &'static str) -> anyhow::Result<usize> {
    res.map_err(|code| anyhow::anyhow!("{ctx} (zstd error code {code:?})"))
}
