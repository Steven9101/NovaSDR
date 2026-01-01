use flacenc::component::{BitRepr, Stream};
use flacenc::config;
use flacenc::error::{Verified, Verify};
use flacenc::source::{Fill, FrameBuf};

pub struct FlacStreamEncoder {
    cfg: Verified<config::Encoder>,
    stream: Stream,
    frame_number: u64,
    block_size: usize,
    frame_buf: FrameBuf,
}

impl FlacStreamEncoder {
    pub fn new(
        sample_rate: usize,
        bits_per_sample: usize,
        block_size: usize,
    ) -> anyhow::Result<Self> {
        let cfg = config::Encoder::default()
            .into_verified()
            .map_err(|e| anyhow::anyhow!("flac config verify: {e:?}"))?;

        let mut stream = Stream::new(sample_rate, 1, bits_per_sample)
            .map_err(|e| anyhow::anyhow!("flac streaminfo: {e:?}"))?;
        stream
            .stream_info_mut()
            .set_block_sizes(block_size, block_size)
            .map_err(|e| anyhow::anyhow!("flac set block sizes: {e:?}"))?;

        let frame_buf = FrameBuf::with_size(1, block_size)
            .map_err(|e| anyhow::anyhow!("flac framebuf: {e:?}"))?;

        Ok(Self {
            cfg,
            stream,
            frame_number: 0,
            block_size,
            frame_buf,
        })
    }

    pub fn header_bytes(&self) -> anyhow::Result<Vec<u8>> {
        let mut sink = flacenc::bitsink::MemSink::<u8>::new();
        self.stream
            .write(&mut sink)
            .map_err(|e| anyhow::anyhow!("flac header write: {e:?}"))?;
        Ok(sink.into_inner())
    }

    pub fn encode_block(&mut self, pcm_i32: &[i32]) -> anyhow::Result<Vec<u8>> {
        anyhow::ensure!(
            pcm_i32.len() == self.block_size,
            "flac block size mismatch (expected {}, got {})",
            self.block_size,
            pcm_i32.len()
        );

        self.frame_buf
            .fill_interleaved(pcm_i32)
            .map_err(|e| anyhow::anyhow!("flac fill interleaved: {e:?}"))?;

        let frame = flacenc::encode_fixed_size_frame(
            &self.cfg,
            &self.frame_buf,
            self.frame_number as usize,
            self.stream.stream_info(),
        )
        .map_err(|e| anyhow::anyhow!("flac encode frame: {e:?}"))?;
        self.frame_number += 1;

        let mut sink = flacenc::bitsink::MemSink::<u8>::new();
        frame
            .write(&mut sink)
            .map_err(|e| anyhow::anyhow!("flac frame write: {e:?}"))?;
        Ok(sink.into_inner())
    }
}
