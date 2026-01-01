use crate::dsp::window::hann_window;
use anyhow::Context;
use num_complex::Complex32;
use opencl3::{
    command_queue::{CommandQueue, CL_BLOCKING, CL_NON_BLOCKING},
    context::Context as ClContext,
    device::{Device, CL_DEVICE_TYPE_ALL, CL_DEVICE_TYPE_GPU},
    kernel::{ExecuteKernel, Kernel},
    memory::{Buffer, ClMem, CL_MEM_READ_WRITE},
    platform::{get_platforms, Platform},
    program::Program,
    types::{cl_command_queue, cl_float, cl_int, cl_mem},
};
use std::sync::OnceLock;

#[allow(non_camel_case_types)]
#[allow(non_snake_case)]
#[allow(non_upper_case_globals)]
mod ffi {
    use opencl3::types::{cl_command_queue, cl_context, cl_event, cl_mem, cl_uint};
    use std::ffi::c_void;

    pub type clfftPlanHandle = usize;

    #[repr(i32)]
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum clfftStatus {
        CLFFT_SUCCESS = 0,
    }

    #[repr(u32)]
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum clfftDim {
        CLFFT_1D = 1,
    }

    #[repr(u32)]
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum clfftPrecision {
        #[allow(dead_code)]
        CLFFT_SINGLE = 1,
    }

    #[repr(u32)]
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum clfftLayout {
        CLFFT_COMPLEX_INTERLEAVED = 1,
        CLFFT_HERMITIAN_INTERLEAVED = 3,
        CLFFT_REAL = 5,
    }

    #[repr(u32)]
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum clfftResultLocation {
        CLFFT_INPLACE = 1,
        CLFFT_OUTOFPLACE = 2,
    }

    #[repr(i32)]
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum clfftDirection {
        CLFFT_FORWARD = -1,
    }

    #[repr(C)]
    #[derive(Copy, Clone)]
    pub struct clfftSetupData {
        pub major: cl_uint,
        pub minor: cl_uint,
        pub patch: cl_uint,
        pub debugFlags: cl_uint,
    }

    #[link(name = "clFFT")]
    extern "C" {
        pub fn clfftGetVersion(
            major: *mut cl_uint,
            minor: *mut cl_uint,
            patch: *mut cl_uint,
        ) -> clfftStatus;
        pub fn clfftSetup(setupData: *const clfftSetupData) -> clfftStatus;
        #[allow(dead_code)]
        pub fn clfftTeardown() -> clfftStatus;

        pub fn clfftCreateDefaultPlan(
            plHandle: *mut clfftPlanHandle,
            context: cl_context,
            dim: clfftDim,
            clLengths: *const usize,
        ) -> clfftStatus;
        pub fn clfftDestroyPlan(plHandle: *mut clfftPlanHandle) -> clfftStatus;
        pub fn clfftSetPlanPrecision(
            plHandle: clfftPlanHandle,
            precision: clfftPrecision,
        ) -> clfftStatus;
        pub fn clfftSetLayout(
            plHandle: clfftPlanHandle,
            iLayout: clfftLayout,
            oLayout: clfftLayout,
        ) -> clfftStatus;
        pub fn clfftSetResultLocation(
            plHandle: clfftPlanHandle,
            placeness: clfftResultLocation,
        ) -> clfftStatus;
        pub fn clfftBakePlan(
            plHandle: clfftPlanHandle,
            numQueues: cl_uint,
            commQueues: *mut cl_command_queue,
            pfn_notify: Option<extern "C" fn(clfftPlanHandle, cl_uint, *mut c_void)>,
            user_data: *mut c_void,
        ) -> clfftStatus;
        pub fn clfftEnqueueTransform(
            plHandle: clfftPlanHandle,
            dir: clfftDirection,
            numQueuesAndEvents: cl_uint,
            commQueues: *mut cl_command_queue,
            numWaitEvents: cl_uint,
            waitEvents: *const cl_event,
            outEvents: *mut cl_event,
            inputBuffers: *mut cl_mem,
            outputBuffers: *mut cl_mem,
            tmpBuffer: cl_mem,
        ) -> clfftStatus;
    }
}

fn ensure_setup() -> anyhow::Result<()> {
    static SETUP: OnceLock<anyhow::Result<()>> = OnceLock::new();
    let res = SETUP.get_or_init(|| unsafe {
        let mut major: u32 = 0;
        let mut minor: u32 = 0;
        let mut patch: u32 = 0;
        let st = ffi::clfftGetVersion(&mut major, &mut minor, &mut patch);
        if st != ffi::clfftStatus::CLFFT_SUCCESS {
            return Err(anyhow::anyhow!("clfftGetVersion failed: {st:?}"));
        }
        let data = ffi::clfftSetupData {
            major,
            minor,
            patch,
            debugFlags: 0,
        };
        let st = ffi::clfftSetup(&data);
        if st != ffi::clfftStatus::CLFFT_SUCCESS {
            return Err(anyhow::anyhow!("clfftSetup failed: {st:?}"));
        }
        Ok(())
    });

    match res {
        Ok(()) => Ok(()),
        Err(e) => Err(anyhow::anyhow!(e.to_string())),
    }
}

pub struct ClfftComplexFft {
    n: usize,
    _ctx: ClContext,
    queue: CommandQueue,
    buf: Buffer<f32>,
    plan: ffi::clfftPlanHandle,
    window_complex: Kernel,
    window_buf: Buffer<f32>,
    waterfall: WaterfallGpuQuantizer,
}

impl ClfftComplexFft {
    pub fn new(n: usize) -> anyhow::Result<Self> {
        ensure_setup()?;

        let (platform_idx, device_idx) = select_indices_from_env()?;
        let (platform, device_id) = select_platform_device(platform_idx, device_idx)?;
        let device = Device::new(device_id);

        let device_name = device.name().unwrap_or_else(|_| "<unknown>".to_string());
        tracing::info!(opencl_device = %device_name, fft_size = n, "clFFT enabled");

        let ctx = ClContext::from_device(&device).context("create OpenCL context")?;
        let queue = unsafe { CommandQueue::create(&ctx, device_id, 0) }
            .context("create OpenCL command queue")?;

        let buf =
            unsafe { Buffer::<f32>::create(&ctx, CL_MEM_READ_WRITE, n * 2, std::ptr::null_mut()) }
                .context("create OpenCL buffer")?;

        // Build kernels for windowing and waterfall quantization.
        let program = Program::create_and_build_from_source(&ctx, WATERFALL_OPENCL_KERNELS, "")
            .map_err(|e| anyhow::anyhow!("build OpenCL program: {e}"))?;
        let window_complex =
            Kernel::create(&program, "window_complex").context("kernel window_complex")?;

        let mut window_buf =
            unsafe { Buffer::<f32>::create(&ctx, CL_MEM_READ_WRITE, n, std::ptr::null_mut()) }
                .context("create OpenCL window buffer")?;
        let window = hann_window(n);
        unsafe {
            queue
                .enqueue_write_buffer(&mut window_buf, CL_BLOCKING, 0, &window, &[])
                .context("OpenCL write window")?;
        }

        let waterfall = WaterfallGpuQuantizer::new(&ctx, n)?;

        let mut plan: ffi::clfftPlanHandle = 0;
        let lengths = [n];
        let st = unsafe {
            ffi::clfftCreateDefaultPlan(
                &mut plan,
                ctx.get(),
                ffi::clfftDim::CLFFT_1D,
                lengths.as_ptr(),
            )
        };
        anyhow::ensure!(
            st == ffi::clfftStatus::CLFFT_SUCCESS,
            "clfftCreateDefaultPlan failed: {st:?}"
        );

        // Prefer correctness and consistent output over "FAST" variants.
        let st = unsafe { ffi::clfftSetPlanPrecision(plan, ffi::clfftPrecision::CLFFT_SINGLE) };
        anyhow::ensure!(
            st == ffi::clfftStatus::CLFFT_SUCCESS,
            "clfftSetPlanPrecision failed: {st:?}"
        );

        let st = unsafe {
            ffi::clfftSetLayout(
                plan,
                ffi::clfftLayout::CLFFT_COMPLEX_INTERLEAVED,
                ffi::clfftLayout::CLFFT_COMPLEX_INTERLEAVED,
            )
        };
        anyhow::ensure!(
            st == ffi::clfftStatus::CLFFT_SUCCESS,
            "clfftSetLayout failed: {st:?}"
        );

        let st =
            unsafe { ffi::clfftSetResultLocation(plan, ffi::clfftResultLocation::CLFFT_INPLACE) };
        anyhow::ensure!(
            st == ffi::clfftStatus::CLFFT_SUCCESS,
            "clfftSetResultLocation failed: {st:?}"
        );

        let mut q: cl_command_queue = queue.get();
        let st = unsafe { ffi::clfftBakePlan(plan, 1, &mut q, None, std::ptr::null_mut()) };
        anyhow::ensure!(
            st == ffi::clfftStatus::CLFFT_SUCCESS,
            "clfftBakePlan failed: {st:?}"
        );

        let _ = platform;

        Ok(Self {
            n,
            _ctx: ctx,
            queue,
            buf,
            plan,
            window_complex,
            window_buf,
            waterfall,
        })
    }

    pub fn window_and_process_inplace(&mut self, data: &[Complex32]) -> anyhow::Result<()> {
        anyhow::ensure!(data.len() == self.n, "clFFT input length mismatch");

        // Upload unwindowed interleaved complex input.
        let interleaved = complex_as_f32_slice(data);
        unsafe {
            self.queue
                .enqueue_write_buffer(&mut self.buf, CL_NON_BLOCKING, 0, interleaved, &[])
                .context("OpenCL write")?;
        }

        // Apply Hann window on GPU (in-place).
        let offset0: cl_int = 0;
        // SAFETY: Kernel args match the OpenCL C signature and buffers are valid for the duration
        // of the enqueue; work size is within allocated buffer bounds.
        unsafe {
            ExecuteKernel::new(&self.window_complex)
                .set_arg(&self.buf)
                .set_arg(&offset0)
                .set_arg(&self.buf)
                .set_arg(&self.window_buf)
                .set_global_work_size(self.n)
                .enqueue_nd_range(&self.queue)?;
        }

        // Run clFFT in-place.
        let mut q: cl_command_queue = self.queue.get();
        let mut mem: cl_mem = self.buf.get();

        let st = unsafe {
            ffi::clfftEnqueueTransform(
                self.plan,
                ffi::clfftDirection::CLFFT_FORWARD,
                1,
                &mut q,
                0,
                std::ptr::null(),
                std::ptr::null_mut(),
                &mut mem,
                std::ptr::null_mut(),
                0 as cl_mem,
            )
        };
        anyhow::ensure!(
            st == ffi::clfftStatus::CLFFT_SUCCESS,
            "clfftEnqueueTransform failed: {st:?}"
        );

        Ok(())
    }

    pub fn read_fft_output(&mut self, out: &mut [Complex32]) -> anyhow::Result<()> {
        anyhow::ensure!(out.len() == self.n, "clFFT output length mismatch");
        let out_interleaved = complex_as_f32_slice_mut(out);
        unsafe {
            self.queue
                .enqueue_read_buffer(&self.buf, CL_BLOCKING, 0, out_interleaved, &[])
                .context("OpenCL read")?;
        }
        Ok(())
    }

    pub fn quantize_and_downsample(
        &mut self,
        base_idx: usize,
        downsample_levels: usize,
        size_log2: i32,
        normalize: f32,
    ) -> anyhow::Result<(Vec<i8>, Vec<usize>)> {
        anyhow::ensure!(downsample_levels >= 1, "downsample_levels must be >= 1");
        self.waterfall.quantize_and_downsample_complexbuf(
            &self.queue,
            &self.buf,
            WaterfallQuantizeArgs {
                outbuf_len: self.n,
                base_idx,
                levels: downsample_levels,
                size_log2,
                normalize,
            },
        )
    }

    pub fn max_power(&mut self) -> anyhow::Result<f32> {
        let power = self.waterfall.read_base_power(&self.queue, self.n)?;
        let mut max_p = 0.0f32;
        for p in power {
            if p.is_finite() && p > max_p {
                max_p = p;
            }
        }
        Ok(max_p)
    }

    pub fn process_inplace(&mut self, data: &mut [Complex32]) -> anyhow::Result<()> {
        anyhow::ensure!(data.len() == self.n, "clFFT input length mismatch");

        let interleaved = complex_as_f32_slice(data);
        unsafe {
            self.queue
                .enqueue_write_buffer(&mut self.buf, CL_NON_BLOCKING, 0, interleaved, &[])
                .context("OpenCL write")?;
        }

        let mut q: cl_command_queue = self.queue.get();
        let mut mem: cl_mem = self.buf.get();

        let st = unsafe {
            ffi::clfftEnqueueTransform(
                self.plan,
                ffi::clfftDirection::CLFFT_FORWARD,
                1,
                &mut q,
                0,
                std::ptr::null(),
                std::ptr::null_mut(),
                &mut mem,
                std::ptr::null_mut(),
                0 as cl_mem,
            )
        };
        anyhow::ensure!(
            st == ffi::clfftStatus::CLFFT_SUCCESS,
            "clfftEnqueueTransform failed: {st:?}"
        );

        let out = complex_as_f32_slice_mut(data);
        unsafe {
            self.queue
                .enqueue_read_buffer(&self.buf, CL_BLOCKING, 0, out, &[])
                .context("OpenCL read")?;
        }
        Ok(())
    }
}

impl Drop for ClfftComplexFft {
    fn drop(&mut self) {
        unsafe {
            let mut plan = self.plan;
            let _ = ffi::clfftDestroyPlan(&mut plan);
        }
    }
}

fn complex_as_f32_slice(v: &[Complex32]) -> &[f32] {
    debug_assert_eq!(std::mem::size_of::<Complex32>(), 8);
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<f32>(), v.len() * 2) }
}

fn complex_as_f32_slice_mut(v: &mut [Complex32]) -> &mut [f32] {
    debug_assert_eq!(std::mem::size_of::<Complex32>(), 8);
    unsafe { std::slice::from_raw_parts_mut(v.as_mut_ptr().cast::<f32>(), v.len() * 2) }
}

const WATERFALL_OPENCL_KERNELS: &str = r#"
    void kernel window_real(global float* restrict outbuf, int offset, global float* restrict inbuf, global const float* restrict windowbuf){
        int i = get_global_id(0);
        outbuf[i + offset] = inbuf[i] * windowbuf[i + offset];
    }
    void kernel window_complex(global float* restrict outbuf, int offset, global float* restrict inbuf, global const float* restrict windowbuf){
        int i = get_global_id(0);
        int i_offset = i + offset;
        outbuf[i_offset*2] = inbuf[i*2] * windowbuf[i_offset];
        outbuf[i_offset*2+1] = inbuf[i*2+1] * windowbuf[i_offset];
    }
    inline char log_power(float power, int power_offset) {
        return convert_char_sat_rtz(20 * log10(power) + 127. + power_offset * 6.020599913279624);
    }
    #pragma OPENCL EXTENSION cl_khr_byte_addressable_store : enable
    void kernel power_and_quantize(global float * restrict complexbuf, global float * restrict powerbuf,
                                   global char * restrict quantizedbuf, float normalize,
                                   int complexbuf_offset, int outputbuf_offset, int power_offset) {
        int i = get_global_id(0);
        int i_complex = i + complexbuf_offset;
        int i_output = i + outputbuf_offset;
        float re = complexbuf[i_complex * 2] / normalize;
        float im = complexbuf[i_complex * 2 + 1] / normalize;
        float power = re * re + im * im;
        powerbuf[i_output] = power;
        quantizedbuf[i_output] = log_power(power, power_offset);
    }
    void kernel half_and_quantize(global const float * restrict powerbuf, global float * restrict halfbuf,
                                  global char * restrict quantizedbuf,
                                  int powerbuf_offset, int outputbuf_offset, int power_offset) {
        int i = get_global_id(0);
        float power = powerbuf[powerbuf_offset + i * 2] + powerbuf[powerbuf_offset + i * 2 + 1];
        halfbuf[i + outputbuf_offset] = power;
        quantizedbuf[i + outputbuf_offset] = log_power(power, power_offset);
    }
"#;

#[derive(Clone, Copy, Debug)]
struct WaterfallQuantizeArgs {
    outbuf_len: usize,
    base_idx: usize,
    levels: usize,
    size_log2: i32,
    normalize: f32,
}

struct WaterfallGpuQuantizer {
    power_and_quantize: Kernel,
    half_and_quantize: Kernel,
    power_buf: Buffer<f32>,
    quantized_buf: Buffer<i8>,
}

impl WaterfallGpuQuantizer {
    fn new(ctx: &ClContext, outbuf_len: usize) -> anyhow::Result<Self> {
        // Total concatenated waterfall length is < 2*outbuf_len.
        let power_buf = unsafe {
            Buffer::<f32>::create(ctx, CL_MEM_READ_WRITE, outbuf_len * 2, std::ptr::null_mut())
        }
        .context("create OpenCL power buffer")?;

        let quantized_buf = unsafe {
            Buffer::<i8>::create(ctx, CL_MEM_READ_WRITE, outbuf_len * 2, std::ptr::null_mut())
        }
        .context("create OpenCL quantized buffer")?;

        let program = Program::create_and_build_from_source(ctx, WATERFALL_OPENCL_KERNELS, "")
            .map_err(|e| anyhow::anyhow!("build OpenCL program: {e}"))?;

        let power_and_quantize =
            Kernel::create(&program, "power_and_quantize").context("kernel power_and_quantize")?;
        let half_and_quantize =
            Kernel::create(&program, "half_and_quantize").context("kernel half_and_quantize")?;

        Ok(Self {
            power_and_quantize,
            half_and_quantize,
            power_buf,
            quantized_buf,
        })
    }

    fn compute_offsets(levels: usize, base_len: usize) -> (Vec<usize>, usize) {
        let mut offsets = Vec::with_capacity(levels);
        let mut cur_offset = 0usize;
        let mut cur_len = base_len;
        for _ in 0..levels {
            offsets.push(cur_offset);
            cur_offset += cur_len;
            cur_len /= 2;
        }
        (offsets, cur_offset)
    }

    fn quantize_and_downsample_complexbuf(
        &mut self,
        queue: &CommandQueue,
        complexbuf: &Buffer<f32>,
        args: WaterfallQuantizeArgs,
    ) -> anyhow::Result<(Vec<i8>, Vec<usize>)> {
        let (offsets, total_len) = Self::compute_offsets(args.levels, args.outbuf_len);

        let norm: cl_float = args.normalize;
        let pow0: cl_int = args.size_log2;

        // Perform a frequency shift by splitting the kernel run into two parts.
        if args.base_idx > 0 && args.base_idx < args.outbuf_len {
            let base_idx_i: cl_int = args.base_idx as cl_int;
            let out0: cl_int = 0;
            let out1: cl_int = (args.outbuf_len - args.base_idx) as cl_int;
            let zero: cl_int = 0;

            unsafe {
                ExecuteKernel::new(&self.power_and_quantize)
                    .set_arg(complexbuf)
                    .set_arg(&self.power_buf)
                    .set_arg(&self.quantized_buf)
                    .set_arg(&norm)
                    .set_arg(&base_idx_i)
                    .set_arg(&out0)
                    .set_arg(&pow0)
                    .set_global_work_size(args.outbuf_len - args.base_idx)
                    .enqueue_nd_range(queue)?;

                ExecuteKernel::new(&self.power_and_quantize)
                    .set_arg(complexbuf)
                    .set_arg(&self.power_buf)
                    .set_arg(&self.quantized_buf)
                    .set_arg(&norm)
                    .set_arg(&zero)
                    .set_arg(&out1)
                    .set_arg(&pow0)
                    .set_global_work_size(args.base_idx)
                    .enqueue_nd_range(queue)?;
            }
        } else {
            let complex_off: cl_int = 0;
            let out_off: cl_int = 0;
            unsafe {
                ExecuteKernel::new(&self.power_and_quantize)
                    .set_arg(complexbuf)
                    .set_arg(&self.power_buf)
                    .set_arg(&self.quantized_buf)
                    .set_arg(&norm)
                    .set_arg(&complex_off)
                    .set_arg(&out_off)
                    .set_arg(&pow0)
                    .set_global_work_size(args.outbuf_len)
                    .enqueue_nd_range(queue)?;
            }
        }

        let mut out_len = args.outbuf_len;
        let mut offset = 0usize;
        for i in 0..args.levels.saturating_sub(1) {
            let pow: cl_int = (args.size_log2 - (i as i32) - 1) as cl_int;
            let powerbuf_offset: cl_int = offset as cl_int;
            let outputbuf_offset: cl_int = (offset + out_len) as cl_int;
            unsafe {
                ExecuteKernel::new(&self.half_and_quantize)
                    .set_arg(&self.power_buf)
                    .set_arg(&self.power_buf)
                    .set_arg(&self.quantized_buf)
                    .set_arg(&powerbuf_offset)
                    .set_arg(&outputbuf_offset)
                    .set_arg(&pow)
                    .set_global_work_size(out_len / 2)
                    .enqueue_nd_range(queue)?;
            }
            offset += out_len;
            out_len /= 2;
        }

        let mut quantized = vec![0i8; total_len];
        unsafe {
            queue
                .enqueue_read_buffer(&self.quantized_buf, CL_BLOCKING, 0, &mut quantized, &[])
                .context("OpenCL read quantized")?;
        }
        Ok((quantized, offsets))
    }

    fn read_base_power(
        &mut self,
        queue: &CommandQueue,
        outbuf_len: usize,
    ) -> anyhow::Result<Vec<f32>> {
        let mut out = vec![0.0f32; outbuf_len];
        unsafe {
            queue
                .enqueue_read_buffer(&self.power_buf, CL_BLOCKING, 0, &mut out, &[])
                .context("OpenCL read power")?;
        }
        Ok(out)
    }
}

pub struct ClfftRealFft {
    n: usize,
    _ctx: ClContext,
    queue: CommandQueue,
    window_real: Kernel,
    window_buf: Buffer<f32>,
    half_a_buf: Buffer<f32>,
    half_b_buf: Buffer<f32>,
    in_buf: Buffer<f32>,
    out_buf: Buffer<f32>,
    waterfall: WaterfallGpuQuantizer,
    plan: ffi::clfftPlanHandle,
}

impl ClfftRealFft {
    pub fn new(n: usize, window: &[f32]) -> anyhow::Result<Self> {
        ensure_setup()?;
        anyhow::ensure!(window.len() == n, "window length mismatch");

        let (platform_idx, device_idx) = select_indices_from_env()?;
        let (platform, device_id) = select_platform_device(platform_idx, device_idx)?;
        let device = Device::new(device_id);

        let device_name = device.name().unwrap_or_else(|_| "<unknown>".to_string());
        tracing::info!(opencl_device = %device_name, fft_size = n, "clFFT real FFT enabled");

        let ctx = ClContext::from_device(&device).context("create OpenCL context")?;
        let queue = unsafe { CommandQueue::create(&ctx, device_id, 0) }
            .context("create OpenCL command queue")?;

        let program = Program::create_and_build_from_source(&ctx, WATERFALL_OPENCL_KERNELS, "")
            .map_err(|e| anyhow::anyhow!("build OpenCL program: {e}"))?;
        let window_real = Kernel::create(&program, "window_real").context("kernel window_real")?;

        let mut window_buf =
            unsafe { Buffer::<f32>::create(&ctx, CL_MEM_READ_WRITE, n, std::ptr::null_mut()) }
                .context("create OpenCL window buffer")?;
        unsafe {
            queue
                .enqueue_write_buffer(&mut window_buf, CL_BLOCKING, 0, window, &[])
                .context("OpenCL write window")?;
        }

        let half_len = n / 2;
        let half_a_buf = unsafe {
            Buffer::<f32>::create(&ctx, CL_MEM_READ_WRITE, half_len, std::ptr::null_mut())
        }
        .context("create OpenCL half_a buffer")?;
        let half_b_buf = unsafe {
            Buffer::<f32>::create(&ctx, CL_MEM_READ_WRITE, half_len, std::ptr::null_mut())
        }
        .context("create OpenCL half_b buffer")?;

        let in_buf =
            unsafe { Buffer::<f32>::create(&ctx, CL_MEM_READ_WRITE, n, std::ptr::null_mut()) }
                .context("create OpenCL input buffer")?;

        // clFFT allocates (N+2) complex values for R2C output.
        let out_len_complex = n + 2;
        let out_buf = unsafe {
            Buffer::<f32>::create(
                &ctx,
                CL_MEM_READ_WRITE,
                out_len_complex * 2,
                std::ptr::null_mut(),
            )
        }
        .context("create OpenCL output buffer")?;

        let waterfall =
            WaterfallGpuQuantizer::new(&ctx, n / 2).context("init waterfall kernels")?;

        let mut plan: ffi::clfftPlanHandle = 0;
        let lengths = [n];
        let st = unsafe {
            ffi::clfftCreateDefaultPlan(
                &mut plan,
                ctx.get(),
                ffi::clfftDim::CLFFT_1D,
                lengths.as_ptr(),
            )
        };
        anyhow::ensure!(
            st == ffi::clfftStatus::CLFFT_SUCCESS,
            "clfftCreateDefaultPlan failed: {st:?}"
        );

        // Prefer correctness and consistent output over "FAST" variants.
        let st = unsafe { ffi::clfftSetPlanPrecision(plan, ffi::clfftPrecision::CLFFT_SINGLE) };
        anyhow::ensure!(
            st == ffi::clfftStatus::CLFFT_SUCCESS,
            "clfftSetPlanPrecision failed: {st:?}"
        );

        let st = unsafe {
            ffi::clfftSetLayout(
                plan,
                ffi::clfftLayout::CLFFT_REAL,
                ffi::clfftLayout::CLFFT_HERMITIAN_INTERLEAVED,
            )
        };
        anyhow::ensure!(
            st == ffi::clfftStatus::CLFFT_SUCCESS,
            "clfftSetLayout failed: {st:?}"
        );

        let st = unsafe {
            ffi::clfftSetResultLocation(plan, ffi::clfftResultLocation::CLFFT_OUTOFPLACE)
        };
        anyhow::ensure!(
            st == ffi::clfftStatus::CLFFT_SUCCESS,
            "clfftSetResultLocation failed: {st:?}"
        );

        let mut q: cl_command_queue = queue.get();
        let st = unsafe { ffi::clfftBakePlan(plan, 1, &mut q, None, std::ptr::null_mut()) };
        anyhow::ensure!(
            st == ffi::clfftStatus::CLFFT_SUCCESS,
            "clfftBakePlan failed: {st:?}"
        );

        let _ = platform;

        Ok(Self {
            n,
            _ctx: ctx,
            queue,
            window_real,
            window_buf,
            half_a_buf,
            half_b_buf,
            in_buf,
            out_buf,
            waterfall,
            plan,
        })
    }

    pub fn load_real_input(&mut self, half_a: &[f32], half_b: &[f32]) -> anyhow::Result<()> {
        let half_len = self.n / 2;
        anyhow::ensure!(
            half_a.len() == half_len,
            "clFFT real half_a length mismatch"
        );
        anyhow::ensure!(
            half_b.len() == half_len,
            "clFFT real half_b length mismatch"
        );

        unsafe {
            self.queue
                .enqueue_write_buffer(&mut self.half_a_buf, CL_NON_BLOCKING, 0, half_a, &[])
                .context("OpenCL write half_a")?;
            self.queue
                .enqueue_write_buffer(&mut self.half_b_buf, CL_NON_BLOCKING, 0, half_b, &[])
                .context("OpenCL write half_b")?;

            let offset0: cl_int = 0;
            ExecuteKernel::new(&self.window_real)
                .set_arg(&self.in_buf)
                .set_arg(&offset0)
                .set_arg(&self.half_a_buf)
                .set_arg(&self.window_buf)
                .set_global_work_size(half_len)
                .enqueue_nd_range(&self.queue)?;

            let offset1: cl_int = half_len as cl_int;
            ExecuteKernel::new(&self.window_real)
                .set_arg(&self.in_buf)
                .set_arg(&offset1)
                .set_arg(&self.half_b_buf)
                .set_arg(&self.window_buf)
                .set_global_work_size(half_len)
                .enqueue_nd_range(&self.queue)?;
        }

        Ok(())
    }

    pub fn process_fft(&mut self, output: &mut [Complex32]) -> anyhow::Result<()> {
        anyhow::ensure!(
            output.len() == (self.n / 2) + 1,
            "clFFT real output length mismatch"
        );

        let mut q: cl_command_queue = self.queue.get();
        let mut in_mem: cl_mem = self.in_buf.get();
        let mut out_mem: cl_mem = self.out_buf.get();

        let st = unsafe {
            ffi::clfftEnqueueTransform(
                self.plan,
                ffi::clfftDirection::CLFFT_FORWARD,
                1,
                &mut q,
                0,
                std::ptr::null(),
                std::ptr::null_mut(),
                &mut in_mem,
                &mut out_mem,
                0 as cl_mem,
            )
        };
        anyhow::ensure!(
            st == ffi::clfftStatus::CLFFT_SUCCESS,
            "clfftEnqueueTransform failed: {st:?}"
        );

        let out_interleaved = complex_as_f32_slice_mut(output);
        unsafe {
            self.queue
                .enqueue_read_buffer(&self.out_buf, CL_BLOCKING, 0, out_interleaved, &[])
                .context("OpenCL read")?;
        }
        Ok(())
    }

    pub fn quantize_and_downsample(
        &mut self,
        downsample_levels: usize,
        size_log2: i32,
        normalize: f32,
    ) -> anyhow::Result<(Vec<i8>, Vec<usize>)> {
        anyhow::ensure!(downsample_levels >= 1, "downsample_levels must be >= 1");
        // Real output bins are [0..N/2).
        self.waterfall.quantize_and_downsample_complexbuf(
            &self.queue,
            &self.out_buf,
            WaterfallQuantizeArgs {
                outbuf_len: self.n / 2,
                base_idx: 0,
                levels: downsample_levels,
                size_log2,
                normalize,
            },
        )
    }
}

impl Drop for ClfftRealFft {
    fn drop(&mut self) {
        unsafe {
            let mut plan = self.plan;
            let _ = ffi::clfftDestroyPlan(&mut plan);
        }
    }
}

fn select_indices_from_env() -> anyhow::Result<(Option<usize>, Option<usize>)> {
    let platform = match std::env::var("NOVASDR_OPENCL_PLATFORM") {
        Ok(v) => Some(v.parse::<usize>().context("NOVASDR_OPENCL_PLATFORM")?),
        Err(_) => None,
    };
    let device = match std::env::var("NOVASDR_OPENCL_DEVICE") {
        Ok(v) => Some(v.parse::<usize>().context("NOVASDR_OPENCL_DEVICE")?),
        Err(_) => None,
    };
    Ok((platform, device))
}

fn select_platform_device(
    platform_idx: Option<usize>,
    device_idx: Option<usize>,
) -> anyhow::Result<(Platform, opencl3::types::cl_device_id)> {
    let platforms = get_platforms().context("list OpenCL platforms")?;
    anyhow::ensure!(!platforms.is_empty(), "no OpenCL platforms found");

    let platform = match platform_idx {
        Some(i) => platforms
            .get(i)
            .copied()
            .with_context(|| format!("OpenCL platform index {i} not found"))?,
        None => platforms[0],
    };

    let devices = platform
        .get_devices(CL_DEVICE_TYPE_ALL)
        .context("list OpenCL devices")?;
    anyhow::ensure!(!devices.is_empty(), "no OpenCL devices found");

    let device = match device_idx {
        Some(i) => *devices
            .get(i)
            .with_context(|| format!("OpenCL device index {i} not found"))?,
        None => {
            let gpus = platform
                .get_devices(CL_DEVICE_TYPE_GPU)
                .context("list OpenCL GPU devices")?;
            *gpus.first().unwrap_or(&devices[0])
        }
    };

    Ok((platform, device))
}
