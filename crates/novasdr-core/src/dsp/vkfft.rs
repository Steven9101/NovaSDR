use anyhow::Context;
use ash::vk;
use ash::vk::Handle;
use naga::back::spv;
use naga::valid::{Capabilities, ValidationFlags, Validator};
use std::ffi::CStr;
use std::ptr::NonNull;

#[allow(clippy::missing_safety_doc)]
mod ffi {
    use std::ffi::c_char;

    #[repr(C)]
    pub struct NovaVkfftPlan {
        _private: [u8; 0],
    }

    extern "C" {
        pub fn novasdr_vkfft_create_plan(
            physical_device_raw: u64,
            device_raw: u64,
            queue_raw: u64,
            command_pool_raw: u64,
            fence_raw: u64,
            buffer_raw: u64,
            buffer_size_bytes: u64,
            fft_size: u32,
            out_result: *mut i32,
        ) -> *mut NovaVkfftPlan;

        pub fn novasdr_vkfft_record_forward(
            plan: *mut NovaVkfftPlan,
            command_buffer_raw: u64,
        ) -> i32;

        pub fn novasdr_vkfft_error_string(code: i32) -> *const c_char;

        pub fn novasdr_vkfft_destroy_plan(plan: *mut NovaVkfftPlan);
    }
}

pub struct VkfftComplexFft {
    instance: ash::Instance,
    device: ash::Device,
    queue: vk::Queue,
    command_pool: vk::CommandPool,
    command_buffer: vk::CommandBuffer,
    fence: vk::Fence,

    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    memory_is_coherent: bool,
    mapped: NonNull<u8>,

    window: MappedBuffer,
    power: MappedBuffer,
    quant: MappedBuffer,

    desc_set_layout: vk::DescriptorSetLayout,
    desc_pool: vk::DescriptorPool,
    desc_set: vk::DescriptorSet,
    pipeline_layout: vk::PipelineLayout,
    pipeline_window: vk::Pipeline,
    pipeline_power: vk::Pipeline,
    pipeline_half: vk::Pipeline,

    plan: NonNull<ffi::NovaVkfftPlan>,
    fft_size: usize,
}

pub struct VkfftWaterfallQuantizer {
    instance: ash::Instance,
    device: ash::Device,
    queue: vk::Queue,
    command_pool: vk::CommandPool,
    command_buffer: vk::CommandBuffer,
    fence: vk::Fence,

    spectrum: MappedBuffer,
    window: MappedBuffer,
    power: MappedBuffer,
    quant: MappedBuffer,

    desc_set_layout: vk::DescriptorSetLayout,
    desc_pool: vk::DescriptorPool,
    desc_set: vk::DescriptorSet,
    pipeline_layout: vk::PipelineLayout,
    pipeline_power: vk::Pipeline,
    pipeline_half: vk::Pipeline,

    fft_size: usize,
}

struct MappedBuffer {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    is_coherent: bool,
    mapped: NonNull<u8>,
    len_bytes: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PushParams {
    len: u32,
    base_idx: u32,
    src_offset: u32,
    dst_offset: u32,
    power_offset: i32,
    _pad0: i32,
    normalize: f32,
    _pad1: f32,
}

impl VkfftComplexFft {
    pub fn new(fft_size: usize) -> anyhow::Result<Self> {
        anyhow::ensure!(fft_size >= 8, "fft_size too small");
        anyhow::ensure!(
            fft_size.is_power_of_two(),
            "vkfft requires power-of-two fft_size"
        );

        let entry = unsafe { ash::Entry::load().context("load Vulkan loader (libvulkan)")? };
        let instance = create_instance(&entry).context("create Vulkan instance")?;

        let (physical, queue_family_index) =
            select_physical_device(&instance).context("select Vulkan device")?;
        let (device, queue) = create_device(&instance, physical, queue_family_index)
            .context("create Vulkan device")?;

        let command_pool = unsafe {
            device
                .create_command_pool(
                    &vk::CommandPoolCreateInfo::default()
                        .queue_family_index(queue_family_index)
                        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                    None,
                )
                .context("create Vulkan command pool")?
        };

        let command_buffer = unsafe {
            let bufs = device
                .allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(command_pool)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(1),
                )
                .context("allocate Vulkan command buffer")?;
            bufs[0]
        };

        let fence = unsafe {
            device
                .create_fence(&vk::FenceCreateInfo::default(), None)
                .context("create Vulkan fence")?
        };

        let bytes = buffer_bytes_for_complex32(fft_size).context("fft buffer size overflow")?;
        let (buffer, memory, memory_is_coherent, mapped) = create_mapped_buffer(
            &instance,
            &device,
            physical,
            bytes,
            vk::BufferUsageFlags::STORAGE_BUFFER
                | vk::BufferUsageFlags::TRANSFER_SRC
                | vk::BufferUsageFlags::TRANSFER_DST,
        )
        .context("create Vulkan FFT buffer")?;

        let window_bytes = buffer_bytes_for_f32(fft_size).context("window buffer size overflow")?;
        let window = create_mapped_buffer_struct(
            &instance,
            &device,
            physical,
            window_bytes,
            vk::BufferUsageFlags::STORAGE_BUFFER,
        )
        .context("create Vulkan window buffer")?;

        let power_bytes =
            buffer_bytes_for_f32(fft_size * 2).context("power buffer size overflow")?;
        let power = create_mapped_buffer_struct(
            &instance,
            &device,
            physical,
            power_bytes,
            vk::BufferUsageFlags::STORAGE_BUFFER,
        )
        .context("create Vulkan power buffer")?;

        let quant_bytes =
            buffer_bytes_for_i32(fft_size * 2).context("quant buffer size overflow")?;
        let quant = create_mapped_buffer_struct(
            &instance,
            &device,
            physical,
            quant_bytes,
            vk::BufferUsageFlags::STORAGE_BUFFER,
        )
        .context("create Vulkan quant buffer")?;

        upload_window(&device, &window, fft_size).context("upload Hann window")?;

        let (desc_set_layout, desc_pool, desc_set) =
            create_descriptor_set(&device, buffer, bytes, &window, &power, &quant)
                .context("create Vulkan descriptor set")?;

        let pipeline_layout = unsafe {
            device
                .create_pipeline_layout(
                    &vk::PipelineLayoutCreateInfo::default()
                        .set_layouts(&[desc_set_layout])
                        .push_constant_ranges(&[vk::PushConstantRange::default()
                            .stage_flags(vk::ShaderStageFlags::COMPUTE)
                            .offset(0)
                            .size(std::mem::size_of::<PushParams>() as u32)]),
                    None,
                )
                .context("create Vulkan pipeline layout")?
        };

        let (pipeline_window, pipeline_power, pipeline_half) =
            create_compute_pipelines(&device, pipeline_layout)
                .context("create Vulkan pipelines")?;

        let mut result_code: i32 = 0;
        let plan_ptr = unsafe {
            ffi::novasdr_vkfft_create_plan(
                physical.as_raw(),
                device.handle().as_raw(),
                queue.as_raw(),
                command_pool.as_raw(),
                fence.as_raw(),
                buffer.as_raw(),
                bytes,
                fft_size as u32,
                &mut result_code as *mut i32,
            )
        };
        let plan = NonNull::new(plan_ptr).with_context(|| {
            format!(
                "initialize VkFFT plan failed: {}",
                vkfft_error_string(result_code)
            )
        })?;

        Ok(Self {
            instance,
            device,
            queue,
            command_pool,
            command_buffer,
            fence,
            buffer,
            memory,
            memory_is_coherent,
            mapped,
            window,
            power,
            quant,
            desc_set_layout,
            desc_pool,
            desc_set,
            pipeline_layout,
            pipeline_window,
            pipeline_power,
            pipeline_half,
            plan,
            fft_size,
        })
    }

    pub fn window_and_process_inplace(
        &mut self,
        data: &[num_complex::Complex32],
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            data.len() == self.fft_size,
            "vkfft input size mismatch (expected {}, got {})",
            self.fft_size,
            data.len()
        );

        let bytes =
            buffer_bytes_for_complex32(self.fft_size).context("fft buffer size overflow")?;
        unsafe {
            std::ptr::copy_nonoverlapping(
                data.as_ptr().cast::<u8>(),
                self.mapped.as_ptr(),
                bytes as usize,
            );
        }
        flush_mapped(&self.device, self.memory, self.memory_is_coherent, bytes)
            .context("flush Vulkan mapped memory (fft buffer)")?;

        unsafe {
            self.device
                .reset_fences(&[self.fence])
                .context("reset Vulkan fence")?;
            self.device
                .reset_command_buffer(self.command_buffer, vk::CommandBufferResetFlags::empty())
                .context("reset Vulkan command buffer")?;
            self.device
                .begin_command_buffer(
                    self.command_buffer,
                    &vk::CommandBufferBeginInfo::default()
                        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
                )
                .context("begin Vulkan command buffer")?;

            self.device.cmd_bind_pipeline(
                self.command_buffer,
                vk::PipelineBindPoint::COMPUTE,
                self.pipeline_window,
            );
            self.device.cmd_bind_descriptor_sets(
                self.command_buffer,
                vk::PipelineBindPoint::COMPUTE,
                self.pipeline_layout,
                0,
                &[self.desc_set],
                &[],
            );

            let params = PushParams {
                len: self.fft_size as u32,
                base_idx: 0,
                src_offset: 0,
                dst_offset: 0,
                power_offset: 0,
                _pad0: 0,
                normalize: 0.0,
                _pad1: 0.0,
            };
            push_constants(
                &self.device,
                self.command_buffer,
                self.pipeline_layout,
                &params,
            );

            let groups = dispatch_groups(self.fft_size as u32, 256);
            self.device.cmd_dispatch(self.command_buffer, groups, 1, 1);

            cmd_buffer_barrier(
                &self.device,
                self.command_buffer,
                self.buffer,
                vk::AccessFlags::SHADER_WRITE,
                vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE,
            );

            let res =
                ffi::novasdr_vkfft_record_forward(self.plan.as_ptr(), self.command_buffer.as_raw());
            if res != 0 {
                anyhow::bail!("VkFFTAppend failed: {}", vkfft_error_string(res));
            }

            self.device
                .end_command_buffer(self.command_buffer)
                .context("end Vulkan command buffer")?;

            let cmd_bufs = [self.command_buffer];
            let submit_info = vk::SubmitInfo::default().command_buffers(&cmd_bufs);
            self.device
                .queue_submit(self.queue, &[submit_info], self.fence)
                .context("submit Vulkan command buffer")?;
            self.device
                .wait_for_fences(&[self.fence], true, u64::MAX)
                .context("wait for Vulkan fence")?;
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
        anyhow::ensure!(
            base_idx < self.fft_size,
            "vkfft base_idx out of range (base_idx={base_idx}, fft_size={})",
            self.fft_size
        );
        anyhow::ensure!(
            normalize.is_finite() && normalize > 0.0,
            "invalid normalize value"
        );

        let (offsets, total_len) = compute_offsets(downsample_levels, self.fft_size);

        unsafe {
            self.device
                .reset_fences(&[self.fence])
                .context("reset Vulkan fence")?;
            self.device
                .reset_command_buffer(self.command_buffer, vk::CommandBufferResetFlags::empty())
                .context("reset Vulkan command buffer")?;
            self.device
                .begin_command_buffer(
                    self.command_buffer,
                    &vk::CommandBufferBeginInfo::default()
                        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
                )
                .context("begin Vulkan command buffer")?;

            self.device.cmd_bind_descriptor_sets(
                self.command_buffer,
                vk::PipelineBindPoint::COMPUTE,
                self.pipeline_layout,
                0,
                &[self.desc_set],
                &[],
            );

            self.device.cmd_bind_pipeline(
                self.command_buffer,
                vk::PipelineBindPoint::COMPUTE,
                self.pipeline_power,
            );
            let params = PushParams {
                len: self.fft_size as u32,
                base_idx: base_idx as u32,
                src_offset: 0,
                dst_offset: 0,
                power_offset: size_log2,
                _pad0: 0,
                normalize,
                _pad1: 0.0,
            };
            push_constants(
                &self.device,
                self.command_buffer,
                self.pipeline_layout,
                &params,
            );
            let groups = dispatch_groups(self.fft_size as u32, 256);
            self.device.cmd_dispatch(self.command_buffer, groups, 1, 1);

            cmd_buffer_barrier(
                &self.device,
                self.command_buffer,
                self.power.buffer,
                vk::AccessFlags::SHADER_WRITE,
                vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE,
            );
            cmd_buffer_barrier(
                &self.device,
                self.command_buffer,
                self.quant.buffer,
                vk::AccessFlags::SHADER_WRITE,
                vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE,
            );

            self.device.cmd_bind_pipeline(
                self.command_buffer,
                vk::PipelineBindPoint::COMPUTE,
                self.pipeline_half,
            );
            let mut cur_len = self.fft_size;
            for level in 1..downsample_levels {
                let next_len = cur_len / 2;
                let params = PushParams {
                    len: next_len as u32,
                    base_idx: 0,
                    src_offset: offsets[level - 1] as u32,
                    dst_offset: offsets[level] as u32,
                    power_offset: size_log2 - (level as i32) - 1,
                    _pad0: 0,
                    normalize: 0.0,
                    _pad1: 0.0,
                };
                push_constants(
                    &self.device,
                    self.command_buffer,
                    self.pipeline_layout,
                    &params,
                );
                let groups = dispatch_groups(next_len as u32, 256);
                self.device.cmd_dispatch(self.command_buffer, groups, 1, 1);

                cmd_buffer_barrier(
                    &self.device,
                    self.command_buffer,
                    self.power.buffer,
                    vk::AccessFlags::SHADER_WRITE,
                    vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE,
                );
                cmd_buffer_barrier(
                    &self.device,
                    self.command_buffer,
                    self.quant.buffer,
                    vk::AccessFlags::SHADER_WRITE,
                    vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE,
                );

                cur_len = next_len;
            }

            self.device
                .end_command_buffer(self.command_buffer)
                .context("end Vulkan command buffer")?;

            let cmd_bufs = [self.command_buffer];
            let submit_info = vk::SubmitInfo::default().command_buffers(&cmd_bufs);
            self.device
                .queue_submit(self.queue, &[submit_info], self.fence)
                .context("submit Vulkan command buffer")?;
            self.device
                .wait_for_fences(&[self.fence], true, u64::MAX)
                .context("wait for Vulkan fence")?;
        }

        invalidate_mapped(
            &self.device,
            self.power.memory,
            self.power.is_coherent,
            (total_len as u64) * 4,
        )
        .context("invalidate Vulkan mapped memory (power buffer)")?;
        invalidate_mapped(
            &self.device,
            self.quant.memory,
            self.quant.is_coherent,
            (total_len as u64) * 4,
        )
        .context("invalidate Vulkan mapped memory (quant buffer)")?;

        let quant_i32 = unsafe {
            std::slice::from_raw_parts(self.quant.mapped.as_ptr().cast::<i32>(), total_len)
        };
        let mut out = vec![0i8; total_len];
        for (dst, &v) in out.iter_mut().zip(quant_i32.iter()) {
            *dst = v.clamp(-128, 127) as i8;
        }
        Ok((out, offsets))
    }

    pub fn max_power(&mut self) -> anyhow::Result<f32> {
        invalidate_mapped(
            &self.device,
            self.power.memory,
            self.power.is_coherent,
            (self.fft_size as u64) * 4,
        )
        .context("invalidate Vulkan mapped memory (power buffer)")?;

        let power = unsafe {
            std::slice::from_raw_parts(self.power.mapped.as_ptr().cast::<f32>(), self.fft_size)
        };
        let mut max_p = 0.0f32;
        for &p in power {
            if p.is_finite() && p > max_p {
                max_p = p;
            }
        }
        Ok(max_p)
    }

    pub fn read_fft_output(&mut self, out: &mut [num_complex::Complex32]) -> anyhow::Result<()> {
        anyhow::ensure!(out.len() == self.fft_size, "vkfft output length mismatch");
        let bytes =
            buffer_bytes_for_complex32(self.fft_size).context("fft buffer size overflow")?;
        invalidate_mapped(&self.device, self.memory, self.memory_is_coherent, bytes)
            .context("invalidate Vulkan mapped memory (fft buffer)")?;
        unsafe {
            std::ptr::copy_nonoverlapping(
                self.mapped.as_ptr(),
                out.as_mut_ptr().cast::<u8>(),
                bytes as usize,
            );
        }
        Ok(())
    }

    pub fn process_inplace(&mut self, data: &mut [num_complex::Complex32]) -> anyhow::Result<()> {
        anyhow::ensure!(
            data.len() == self.fft_size,
            "vkfft input size mismatch (expected {}, got {})",
            self.fft_size,
            data.len()
        );

        let bytes = (self.fft_size * std::mem::size_of::<num_complex::Complex32>()) as u64;

        unsafe {
            std::ptr::copy_nonoverlapping(
                data.as_ptr().cast::<u8>(),
                self.mapped.as_ptr(),
                bytes as usize,
            );
        }

        if !self.memory_is_coherent {
            unsafe {
                self.device
                    .flush_mapped_memory_ranges(&[vk::MappedMemoryRange::default()
                        .memory(self.memory)
                        .offset(0)
                        .size(bytes)])
                    .context("flush Vulkan mapped memory")?;
            }
        }

        unsafe {
            self.device
                .reset_fences(&[self.fence])
                .context("reset Vulkan fence")?;

            self.device
                .reset_command_buffer(self.command_buffer, vk::CommandBufferResetFlags::empty())
                .context("reset Vulkan command buffer")?;

            self.device
                .begin_command_buffer(
                    self.command_buffer,
                    &vk::CommandBufferBeginInfo::default()
                        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
                )
                .context("begin Vulkan command buffer")?;

            let res =
                ffi::novasdr_vkfft_record_forward(self.plan.as_ptr(), self.command_buffer.as_raw());
            if res != 0 {
                anyhow::bail!("VkFFTAppend failed: {}", vkfft_error_string(res));
            }

            self.device
                .end_command_buffer(self.command_buffer)
                .context("end Vulkan command buffer")?;

            let cmd_bufs = [self.command_buffer];
            let submit_info = vk::SubmitInfo::default().command_buffers(&cmd_bufs);
            self.device
                .queue_submit(self.queue, &[submit_info], self.fence)
                .context("submit Vulkan command buffer")?;

            self.device
                .wait_for_fences(&[self.fence], true, u64::MAX)
                .context("wait for Vulkan fence")?;
        }

        if !self.memory_is_coherent {
            unsafe {
                self.device
                    .invalidate_mapped_memory_ranges(&[vk::MappedMemoryRange::default()
                        .memory(self.memory)
                        .offset(0)
                        .size(bytes)])
                    .context("invalidate Vulkan mapped memory")?;
            }
        }

        unsafe {
            std::ptr::copy_nonoverlapping(
                self.mapped.as_ptr(),
                data.as_mut_ptr().cast::<u8>(),
                bytes as usize,
            );
        }

        Ok(())
    }
}

impl VkfftWaterfallQuantizer {
    pub fn new(fft_size: usize) -> anyhow::Result<Self> {
        anyhow::ensure!(fft_size >= 8, "fft_size too small");
        anyhow::ensure!(
            fft_size.is_power_of_two(),
            "vkfft quantizer requires power-of-two fft_size"
        );

        let entry = unsafe { ash::Entry::load().context("load Vulkan loader (libvulkan)")? };
        let instance = create_instance(&entry).context("create Vulkan instance")?;

        let (physical, queue_family_index) =
            select_physical_device(&instance).context("select Vulkan device")?;
        let (device, queue) = create_device(&instance, physical, queue_family_index)
            .context("create Vulkan device")?;

        let command_pool = unsafe {
            device
                .create_command_pool(
                    &vk::CommandPoolCreateInfo::default()
                        .queue_family_index(queue_family_index)
                        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                    None,
                )
                .context("create Vulkan command pool")?
        };

        let command_buffer = unsafe {
            let bufs = device
                .allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(command_pool)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(1),
                )
                .context("allocate Vulkan command buffer")?;
            bufs[0]
        };

        let fence = unsafe {
            device
                .create_fence(&vk::FenceCreateInfo::default(), None)
                .context("create Vulkan fence")?
        };

        let spectrum_bytes =
            buffer_bytes_for_complex32(fft_size).context("spectrum buffer size overflow")?;
        let spectrum = create_mapped_buffer_struct(
            &instance,
            &device,
            physical,
            spectrum_bytes,
            vk::BufferUsageFlags::STORAGE_BUFFER,
        )
        .context("create Vulkan spectrum buffer")?;

        let window_bytes = buffer_bytes_for_f32(fft_size).context("window buffer size overflow")?;
        let window = create_mapped_buffer_struct(
            &instance,
            &device,
            physical,
            window_bytes,
            vk::BufferUsageFlags::STORAGE_BUFFER,
        )
        .context("create Vulkan window buffer")?;

        let power_bytes =
            buffer_bytes_for_f32(fft_size * 2).context("power buffer size overflow")?;
        let power = create_mapped_buffer_struct(
            &instance,
            &device,
            physical,
            power_bytes,
            vk::BufferUsageFlags::STORAGE_BUFFER,
        )
        .context("create Vulkan power buffer")?;

        let quant_bytes =
            buffer_bytes_for_i32(fft_size * 2).context("quant buffer size overflow")?;
        let quant = create_mapped_buffer_struct(
            &instance,
            &device,
            physical,
            quant_bytes,
            vk::BufferUsageFlags::STORAGE_BUFFER,
        )
        .context("create Vulkan quant buffer")?;

        let (desc_set_layout, desc_pool, desc_set) = create_descriptor_set(
            &device,
            spectrum.buffer,
            spectrum_bytes,
            &window,
            &power,
            &quant,
        )
        .context("create Vulkan descriptor set")?;

        let pipeline_layout = unsafe {
            device
                .create_pipeline_layout(
                    &vk::PipelineLayoutCreateInfo::default()
                        .set_layouts(&[desc_set_layout])
                        .push_constant_ranges(&[vk::PushConstantRange::default()
                            .stage_flags(vk::ShaderStageFlags::COMPUTE)
                            .offset(0)
                            .size(std::mem::size_of::<PushParams>() as u32)]),
                    None,
                )
                .context("create Vulkan pipeline layout")?
        };

        let (pipeline_power, pipeline_half) =
            create_compute_pipelines_quantizer(&device, pipeline_layout)
                .context("create Vulkan pipelines")?;

        Ok(Self {
            instance,
            device,
            queue,
            command_pool,
            command_buffer,
            fence,
            spectrum,
            window,
            power,
            quant,
            desc_set_layout,
            desc_pool,
            desc_set,
            pipeline_layout,
            pipeline_power,
            pipeline_half,
            fft_size,
        })
    }

    pub fn quantize_and_downsample(
        &mut self,
        spectrum: &[num_complex::Complex32],
        base_idx: usize,
        downsample_levels: usize,
        size_log2: i32,
        normalize: f32,
    ) -> anyhow::Result<(Vec<i8>, Vec<usize>)> {
        anyhow::ensure!(spectrum.len() == self.fft_size, "spectrum length mismatch");
        anyhow::ensure!(downsample_levels >= 1, "downsample_levels must be >= 1");
        anyhow::ensure!(
            base_idx < self.fft_size,
            "vkfft base_idx out of range (base_idx={base_idx}, fft_size={})",
            self.fft_size
        );
        anyhow::ensure!(
            normalize.is_finite() && normalize > 0.0,
            "invalid normalize value"
        );

        let spectrum_bytes =
            buffer_bytes_for_complex32(self.fft_size).context("spectrum buffer size overflow")?;
        unsafe {
            std::ptr::copy_nonoverlapping(
                spectrum.as_ptr().cast::<u8>(),
                self.spectrum.mapped.as_ptr(),
                spectrum_bytes as usize,
            );
        }
        flush_mapped(
            &self.device,
            self.spectrum.memory,
            self.spectrum.is_coherent,
            spectrum_bytes,
        )
        .context("flush Vulkan mapped memory (spectrum buffer)")?;

        let (offsets, total_len) = compute_offsets(downsample_levels, self.fft_size);

        unsafe {
            self.device
                .reset_fences(&[self.fence])
                .context("reset Vulkan fence")?;
            self.device
                .reset_command_buffer(self.command_buffer, vk::CommandBufferResetFlags::empty())
                .context("reset Vulkan command buffer")?;
            self.device
                .begin_command_buffer(
                    self.command_buffer,
                    &vk::CommandBufferBeginInfo::default()
                        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
                )
                .context("begin Vulkan command buffer")?;

            self.device.cmd_bind_descriptor_sets(
                self.command_buffer,
                vk::PipelineBindPoint::COMPUTE,
                self.pipeline_layout,
                0,
                &[self.desc_set],
                &[],
            );

            self.device.cmd_bind_pipeline(
                self.command_buffer,
                vk::PipelineBindPoint::COMPUTE,
                self.pipeline_power,
            );

            let params = PushParams {
                len: self.fft_size as u32,
                base_idx: base_idx as u32,
                src_offset: 0,
                dst_offset: 0,
                power_offset: size_log2,
                _pad0: 0,
                normalize,
                _pad1: 0.0,
            };
            push_constants(
                &self.device,
                self.command_buffer,
                self.pipeline_layout,
                &params,
            );
            let groups = dispatch_groups(self.fft_size as u32, 256);
            self.device.cmd_dispatch(self.command_buffer, groups, 1, 1);

            cmd_buffer_barrier(
                &self.device,
                self.command_buffer,
                self.power.buffer,
                vk::AccessFlags::SHADER_WRITE,
                vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE,
            );
            cmd_buffer_barrier(
                &self.device,
                self.command_buffer,
                self.quant.buffer,
                vk::AccessFlags::SHADER_WRITE,
                vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE,
            );

            self.device.cmd_bind_pipeline(
                self.command_buffer,
                vk::PipelineBindPoint::COMPUTE,
                self.pipeline_half,
            );
            let mut cur_len = self.fft_size;
            for level in 1..downsample_levels {
                let next_len = cur_len / 2;
                let params = PushParams {
                    len: next_len as u32,
                    base_idx: 0,
                    src_offset: offsets[level - 1] as u32,
                    dst_offset: offsets[level] as u32,
                    power_offset: size_log2 - (level as i32) - 1,
                    _pad0: 0,
                    normalize: 0.0,
                    _pad1: 0.0,
                };
                push_constants(
                    &self.device,
                    self.command_buffer,
                    self.pipeline_layout,
                    &params,
                );
                let groups = dispatch_groups(next_len as u32, 256);
                self.device.cmd_dispatch(self.command_buffer, groups, 1, 1);

                cmd_buffer_barrier(
                    &self.device,
                    self.command_buffer,
                    self.power.buffer,
                    vk::AccessFlags::SHADER_WRITE,
                    vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE,
                );
                cmd_buffer_barrier(
                    &self.device,
                    self.command_buffer,
                    self.quant.buffer,
                    vk::AccessFlags::SHADER_WRITE,
                    vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE,
                );

                cur_len = next_len;
            }

            self.device
                .end_command_buffer(self.command_buffer)
                .context("end Vulkan command buffer")?;

            let cmd_bufs = [self.command_buffer];
            let submit_info = vk::SubmitInfo::default().command_buffers(&cmd_bufs);
            self.device
                .queue_submit(self.queue, &[submit_info], self.fence)
                .context("submit Vulkan command buffer")?;
            self.device
                .wait_for_fences(&[self.fence], true, u64::MAX)
                .context("wait for Vulkan fence")?;
        }

        invalidate_mapped(
            &self.device,
            self.quant.memory,
            self.quant.is_coherent,
            (total_len as u64) * 4,
        )
        .context("invalidate Vulkan mapped memory (quant buffer)")?;

        let quant_i32 = unsafe {
            std::slice::from_raw_parts(self.quant.mapped.as_ptr().cast::<i32>(), total_len)
        };
        let mut out = vec![0i8; total_len];
        for (dst, &v) in out.iter_mut().zip(quant_i32.iter()) {
            *dst = v.clamp(-128, 127) as i8;
        }
        Ok((out, offsets))
    }
}

impl Drop for VkfftComplexFft {
    fn drop(&mut self) {
        unsafe {
            ffi::novasdr_vkfft_destroy_plan(self.plan.as_ptr());
        }

        unsafe {
            let _ = self.device.device_wait_idle();

            self.device.destroy_pipeline(self.pipeline_half, None);
            self.device.destroy_pipeline(self.pipeline_power, None);
            self.device.destroy_pipeline(self.pipeline_window, None);
            self.device
                .destroy_pipeline_layout(self.pipeline_layout, None);
            self.device.destroy_descriptor_pool(self.desc_pool, None);
            self.device
                .destroy_descriptor_set_layout(self.desc_set_layout, None);

            destroy_mapped_buffer(&self.device, &self.quant);
            destroy_mapped_buffer(&self.device, &self.power);
            destroy_mapped_buffer(&self.device, &self.window);

            self.device.unmap_memory(self.memory);
            self.device.destroy_buffer(self.buffer, None);
            self.device.free_memory(self.memory, None);
            self.device.destroy_fence(self.fence, None);
            self.device
                .free_command_buffers(self.command_pool, &[self.command_buffer]);
            self.device.destroy_command_pool(self.command_pool, None);
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}

impl Drop for VkfftWaterfallQuantizer {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device_wait_idle();

            self.device.destroy_pipeline(self.pipeline_half, None);
            self.device.destroy_pipeline(self.pipeline_power, None);
            self.device
                .destroy_pipeline_layout(self.pipeline_layout, None);
            self.device.destroy_descriptor_pool(self.desc_pool, None);
            self.device
                .destroy_descriptor_set_layout(self.desc_set_layout, None);

            destroy_mapped_buffer(&self.device, &self.quant);
            destroy_mapped_buffer(&self.device, &self.power);
            destroy_mapped_buffer(&self.device, &self.window);
            destroy_mapped_buffer(&self.device, &self.spectrum);

            self.device
                .free_command_buffers(self.command_pool, &[self.command_buffer]);
            self.device.destroy_command_pool(self.command_pool, None);
            self.device.destroy_fence(self.fence, None);
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}

fn vkfft_error_string(code: i32) -> String {
    unsafe {
        let ptr = ffi::novasdr_vkfft_error_string(code);
        if ptr.is_null() {
            return format!("VkFFT error {code}");
        }
        CStr::from_ptr(ptr).to_string_lossy().to_string()
    }
}

fn create_instance(entry: &ash::Entry) -> anyhow::Result<ash::Instance> {
    let app_info = vk::ApplicationInfo::default()
        .application_name(cstr_novasdr())
        .application_version(0)
        .engine_name(cstr_novasdr())
        .engine_version(0)
        .api_version(vk::make_api_version(0, 1, 1, 0));

    let info = vk::InstanceCreateInfo::default().application_info(&app_info);
    unsafe {
        entry
            .create_instance(&info, None)
            .context("vkCreateInstance")
    }
}

fn select_physical_device(instance: &ash::Instance) -> anyhow::Result<(vk::PhysicalDevice, u32)> {
    let devices = unsafe {
        instance
            .enumerate_physical_devices()
            .context("vkEnumeratePhysicalDevices")?
    };
    anyhow::ensure!(!devices.is_empty(), "no Vulkan physical devices found");

    let preferred = std::env::var("NOVASDR_VULKAN_DEVICE")
        .ok()
        .and_then(|s| s.parse::<usize>().ok());

    let mut candidates: Vec<(vk::PhysicalDevice, vk::PhysicalDeviceProperties)> = Vec::new();
    for d in devices {
        let props = unsafe { instance.get_physical_device_properties(d) };
        candidates.push((d, props));
    }

    if let Some(idx) = preferred {
        let (d, _) = candidates
            .get(idx)
            .copied()
            .with_context(|| format!("NOVASDR_VULKAN_DEVICE={idx} out of range"))?;
        let q = find_compute_queue_family(instance, d).context("find compute queue family")?;
        return Ok((d, q));
    }

    // Prefer discrete, then integrated, then anything.
    fn score(p: &vk::PhysicalDeviceProperties) -> i32 {
        match p.device_type {
            vk::PhysicalDeviceType::DISCRETE_GPU => 3,
            vk::PhysicalDeviceType::INTEGRATED_GPU => 2,
            vk::PhysicalDeviceType::VIRTUAL_GPU => 1,
            _ => 0,
        }
    }

    candidates.sort_by_key(|(_, p)| -score(p));
    for (d, _) in candidates {
        if let Ok(q) = find_compute_queue_family(instance, d) {
            return Ok((d, q));
        }
    }

    anyhow::bail!("no Vulkan compute-capable queue family found");
}

fn find_compute_queue_family(
    instance: &ash::Instance,
    device: vk::PhysicalDevice,
) -> anyhow::Result<u32> {
    let families = unsafe { instance.get_physical_device_queue_family_properties(device) };
    for (idx, f) in families.iter().enumerate() {
        if f.queue_count == 0 {
            continue;
        }
        if f.queue_flags.contains(vk::QueueFlags::COMPUTE) {
            return Ok(idx as u32);
        }
    }
    anyhow::bail!("no compute queue family found");
}

fn create_device(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    queue_family_index: u32,
) -> anyhow::Result<(ash::Device, vk::Queue)> {
    let priorities = [1.0f32];
    let queue_info = vk::DeviceQueueCreateInfo::default()
        .queue_family_index(queue_family_index)
        .queue_priorities(&priorities);

    let device = unsafe {
        instance
            .create_device(
                physical,
                &vk::DeviceCreateInfo::default().queue_create_infos(&[queue_info]),
                None,
            )
            .context("vkCreateDevice")?
    };
    let queue = unsafe { device.get_device_queue(queue_family_index, 0) };
    Ok((device, queue))
}

fn allocate_host_visible_memory(
    instance: &ash::Instance,
    device: &ash::Device,
    physical: vk::PhysicalDevice,
    requirements: vk::MemoryRequirements,
) -> anyhow::Result<(vk::DeviceMemory, bool)> {
    let props = unsafe { instance.get_physical_device_memory_properties(physical) };

    let mut pick = None;
    for i in 0..props.memory_type_count {
        let i = i as usize;
        let mt = props.memory_types[i];
        let supported = (requirements.memory_type_bits & (1 << i)) != 0;
        if !supported {
            continue;
        }
        let flags = mt.property_flags;
        if flags.contains(vk::MemoryPropertyFlags::HOST_VISIBLE)
            && flags.contains(vk::MemoryPropertyFlags::HOST_COHERENT)
        {
            pick = Some((i as u32, true));
            break;
        }
        if flags.contains(vk::MemoryPropertyFlags::HOST_VISIBLE) && pick.is_none() {
            pick = Some((i as u32, false));
        }
    }

    let (memory_type_index, coherent) = pick.context("no HOST_VISIBLE Vulkan memory type found")?;

    let alloc = vk::MemoryAllocateInfo::default()
        .allocation_size(requirements.size)
        .memory_type_index(memory_type_index);
    let mem = unsafe {
        device
            .allocate_memory(&alloc, None)
            .context("vkAllocateMemory")?
    };
    Ok((mem, coherent))
}

fn cstr_novasdr() -> &'static CStr {
    // Safety: string is NUL-terminated and static.
    unsafe { CStr::from_bytes_with_nul_unchecked(b"NovaSDR\0") }
}

fn cstr_main() -> &'static CStr {
    unsafe { CStr::from_bytes_with_nul_unchecked(b"main\0") }
}

fn dispatch_groups(len: u32, workgroup: u32) -> u32 {
    (len + workgroup - 1) / workgroup
}

fn push_constants(
    device: &ash::Device,
    command_buffer: vk::CommandBuffer,
    pipeline_layout: vk::PipelineLayout,
    params: &PushParams,
) {
    unsafe {
        let params_bytes = std::slice::from_raw_parts(
            (params as *const PushParams).cast::<u8>(),
            std::mem::size_of::<PushParams>(),
        );
        device.cmd_push_constants(
            command_buffer,
            pipeline_layout,
            vk::ShaderStageFlags::COMPUTE,
            0,
            params_bytes,
        );
    }
}

fn buffer_bytes_for_complex32(len: usize) -> anyhow::Result<u64> {
    Ok(len
        .checked_mul(std::mem::size_of::<num_complex::Complex32>())
        .context("buffer size overflow")? as u64)
}

fn buffer_bytes_for_f32(len: usize) -> anyhow::Result<u64> {
    Ok(len
        .checked_mul(std::mem::size_of::<f32>())
        .context("buffer size overflow")? as u64)
}

fn buffer_bytes_for_i32(len: usize) -> anyhow::Result<u64> {
    Ok(len
        .checked_mul(std::mem::size_of::<i32>())
        .context("buffer size overflow")? as u64)
}

fn create_mapped_buffer(
    instance: &ash::Instance,
    device: &ash::Device,
    physical: vk::PhysicalDevice,
    size: u64,
    usage: vk::BufferUsageFlags,
) -> anyhow::Result<(vk::Buffer, vk::DeviceMemory, bool, NonNull<u8>)> {
    let buffer = unsafe {
        device
            .create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(size)
                    .usage(usage)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )
            .context("create Vulkan buffer")?
    };

    let mem_req = unsafe { device.get_buffer_memory_requirements(buffer) };
    let (memory, coherent) = allocate_host_visible_memory(instance, device, physical, mem_req)
        .context("allocate Vulkan buffer memory")?;

    unsafe {
        device
            .bind_buffer_memory(buffer, memory, 0)
            .context("bind Vulkan buffer memory")?;
    }

    let mapped = unsafe {
        let ptr = device
            .map_memory(memory, 0, size, vk::MemoryMapFlags::empty())
            .context("map Vulkan buffer memory")?;
        NonNull::new(ptr.cast::<u8>()).context("map Vulkan memory returned null")?
    };

    Ok((buffer, memory, coherent, mapped))
}

fn create_mapped_buffer_struct(
    instance: &ash::Instance,
    device: &ash::Device,
    physical: vk::PhysicalDevice,
    size: u64,
    usage: vk::BufferUsageFlags,
) -> anyhow::Result<MappedBuffer> {
    let (buffer, memory, coherent, mapped) =
        create_mapped_buffer(instance, device, physical, size, usage)?;
    Ok(MappedBuffer {
        buffer,
        memory,
        is_coherent: coherent,
        mapped,
        len_bytes: size,
    })
}

fn destroy_mapped_buffer(device: &ash::Device, buf: &MappedBuffer) {
    unsafe {
        device.unmap_memory(buf.memory);
        device.destroy_buffer(buf.buffer, None);
        device.free_memory(buf.memory, None);
    }
}

fn flush_mapped(
    device: &ash::Device,
    memory: vk::DeviceMemory,
    is_coherent: bool,
    size: u64,
) -> anyhow::Result<()> {
    if is_coherent {
        return Ok(());
    }
    unsafe {
        device
            .flush_mapped_memory_ranges(&[vk::MappedMemoryRange::default()
                .memory(memory)
                .offset(0)
                .size(size)])
            .context("flush Vulkan mapped memory")
    }
}

fn invalidate_mapped(
    device: &ash::Device,
    memory: vk::DeviceMemory,
    is_coherent: bool,
    size: u64,
) -> anyhow::Result<()> {
    if is_coherent {
        return Ok(());
    }
    unsafe {
        device
            .invalidate_mapped_memory_ranges(&[vk::MappedMemoryRange::default()
                .memory(memory)
                .offset(0)
                .size(size)])
            .context("invalidate Vulkan mapped memory")
    }
}

fn upload_window(device: &ash::Device, window: &MappedBuffer, n: usize) -> anyhow::Result<()> {
    let window_values = crate::dsp::window::hann_window(n);
    anyhow::ensure!(
        (window_values.len() as u64) * 4 <= window.len_bytes,
        "window buffer too small"
    );

    unsafe {
        std::ptr::copy_nonoverlapping(
            window_values.as_ptr().cast::<u8>(),
            window.mapped.as_ptr(),
            window_values.len() * 4,
        );
    }
    flush_mapped(device, window.memory, window.is_coherent, window.len_bytes)
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

fn cmd_buffer_barrier(
    device: &ash::Device,
    command_buffer: vk::CommandBuffer,
    buffer: vk::Buffer,
    src_access: vk::AccessFlags,
    dst_access: vk::AccessFlags,
) {
    unsafe {
        device.cmd_pipeline_barrier(
            command_buffer,
            vk::PipelineStageFlags::COMPUTE_SHADER,
            vk::PipelineStageFlags::COMPUTE_SHADER,
            vk::DependencyFlags::empty(),
            &[],
            &[vk::BufferMemoryBarrier::default()
                .src_access_mask(src_access)
                .dst_access_mask(dst_access)
                .buffer(buffer)
                .offset(0)
                .size(vk::WHOLE_SIZE)],
            &[],
        );
    }
}

fn create_descriptor_set(
    device: &ash::Device,
    fft_buffer: vk::Buffer,
    fft_bytes: u64,
    window: &MappedBuffer,
    power: &MappedBuffer,
    quant: &MappedBuffer,
) -> anyhow::Result<(
    vk::DescriptorSetLayout,
    vk::DescriptorPool,
    vk::DescriptorSet,
)> {
    unsafe {
        let bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
            vk::DescriptorSetLayoutBinding::default()
                .binding(2)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
            vk::DescriptorSetLayoutBinding::default()
                .binding(3)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
        ];

        let layout = device
            .create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                None,
            )
            .context("create Vulkan descriptor set layout")?;

        let pool_sizes = [vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(4)];
        let pool = device
            .create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .max_sets(1)
                    .pool_sizes(&pool_sizes),
                None,
            )
            .context("create Vulkan descriptor pool")?;

        let sets = device
            .allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(pool)
                    .set_layouts(&[layout]),
            )
            .context("allocate Vulkan descriptor set")?;
        let set = sets[0];

        let infos = [
            vk::DescriptorBufferInfo::default()
                .buffer(fft_buffer)
                .offset(0)
                .range(fft_bytes),
            vk::DescriptorBufferInfo::default()
                .buffer(window.buffer)
                .offset(0)
                .range(window.len_bytes),
            vk::DescriptorBufferInfo::default()
                .buffer(power.buffer)
                .offset(0)
                .range(power.len_bytes),
            vk::DescriptorBufferInfo::default()
                .buffer(quant.buffer)
                .offset(0)
                .range(quant.len_bytes),
        ];

        let writes = [
            vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&infos[0..1]),
            vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(1)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&infos[1..2]),
            vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(2)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&infos[2..3]),
            vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(3)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&infos[3..4]),
        ];
        device.update_descriptor_sets(&writes, &[]);

        Ok((layout, pool, set))
    }
}

fn create_compute_pipelines(
    device: &ash::Device,
    pipeline_layout: vk::PipelineLayout,
) -> anyhow::Result<(vk::Pipeline, vk::Pipeline, vk::Pipeline)> {
    let window_spv = compile_wgsl_to_spirv(include_str!("vkfft_shaders/window.wgsl"), "main")?;
    let power_spv =
        compile_wgsl_to_spirv(include_str!("vkfft_shaders/power_quantize.wgsl"), "main")?;
    let half_spv = compile_wgsl_to_spirv(include_str!("vkfft_shaders/half_quantize.wgsl"), "main")?;

    let window_module = unsafe {
        device
            .create_shader_module(
                &vk::ShaderModuleCreateInfo::default().code(&window_spv),
                None,
            )
            .context("create Vulkan shader module (window)")?
    };
    let power_module = unsafe {
        device
            .create_shader_module(
                &vk::ShaderModuleCreateInfo::default().code(&power_spv),
                None,
            )
            .context("create Vulkan shader module (power)")?
    };
    let half_module = unsafe {
        device
            .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&half_spv), None)
            .context("create Vulkan shader module (half)")?
    };

    let entry = cstr_main();
    let mk_pipeline = |module: vk::ShaderModule| -> anyhow::Result<vk::Pipeline> {
        unsafe {
            let stage = vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::COMPUTE)
                .module(module)
                .name(entry);
            let info = vk::ComputePipelineCreateInfo::default()
                .stage(stage)
                .layout(pipeline_layout);
            let pipelines = device
                .create_compute_pipelines(vk::PipelineCache::null(), &[info], None)
                .map_err(|(_, e)| e)
                .context("create Vulkan compute pipeline")?;
            Ok(pipelines[0])
        }
    };

    let pipeline_window = mk_pipeline(window_module).context("pipeline window")?;
    let pipeline_power = mk_pipeline(power_module).context("pipeline power")?;
    let pipeline_half = mk_pipeline(half_module).context("pipeline half")?;

    unsafe {
        device.destroy_shader_module(half_module, None);
        device.destroy_shader_module(power_module, None);
        device.destroy_shader_module(window_module, None);
    }

    Ok((pipeline_window, pipeline_power, pipeline_half))
}

fn create_compute_pipelines_quantizer(
    device: &ash::Device,
    pipeline_layout: vk::PipelineLayout,
) -> anyhow::Result<(vk::Pipeline, vk::Pipeline)> {
    let power_spv =
        compile_wgsl_to_spirv(include_str!("vkfft_shaders/power_quantize.wgsl"), "main")?;
    let half_spv = compile_wgsl_to_spirv(include_str!("vkfft_shaders/half_quantize.wgsl"), "main")?;

    let power_module = unsafe {
        device
            .create_shader_module(
                &vk::ShaderModuleCreateInfo::default().code(&power_spv),
                None,
            )
            .context("create Vulkan shader module (power)")?
    };
    let half_module = unsafe {
        device
            .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&half_spv), None)
            .context("create Vulkan shader module (half)")?
    };

    let entry = cstr_main();
    let stages = [
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::COMPUTE)
            .module(power_module)
            .name(entry),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::COMPUTE)
            .module(half_module)
            .name(entry),
    ];

    let infos = [
        vk::ComputePipelineCreateInfo::default()
            .stage(stages[0])
            .layout(pipeline_layout),
        vk::ComputePipelineCreateInfo::default()
            .stage(stages[1])
            .layout(pipeline_layout),
    ];
    let pipelines = unsafe {
        device
            .create_compute_pipelines(vk::PipelineCache::null(), &infos, None)
            .map_err(|(_, e)| e)
            .context("vkCreateComputePipelines")?
    };

    unsafe {
        device.destroy_shader_module(power_module, None);
        device.destroy_shader_module(half_module, None);
    }

    Ok((pipelines[0], pipelines[1]))
}

fn compile_wgsl_to_spirv(source: &str, entry_point: &str) -> anyhow::Result<Vec<u32>> {
    let module = naga::front::wgsl::parse_str(source).map_err(|e| anyhow::anyhow!(e))?;
    let mut validator = Validator::new(ValidationFlags::all(), Capabilities::all());
    let info = validator
        .validate(&module)
        .map_err(|e| anyhow::anyhow!("WGSL validation failed: {e}"))?;

    let mut options = spv::Options::default();
    options.lang_version = (1, 0);

    let pipeline_options = spv::PipelineOptions {
        entry_point: entry_point.to_string(),
        shader_stage: naga::ShaderStage::Compute,
    };
    spv::write_vec(&module, &info, &options, Some(&pipeline_options))
        .map_err(|e| anyhow::anyhow!("SPIR-V emit failed: {e}"))
}
