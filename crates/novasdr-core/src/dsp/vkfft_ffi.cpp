// VkFFT wrapper for NovaSDR (Vulkan backend).
//
// This file intentionally exposes a tiny C ABI surface so the Rust code can keep all Vulkan
// resource ownership and just ask VkFFT to record FFT dispatches into a command buffer.
//
// Build is wired through `crates/novasdr-core/build.rs` when the `vkfft` feature is enabled.

#include <stdint.h>
#include <new>

#ifndef VKFFT_BACKEND
#define VKFFT_BACKEND 0
#endif

#ifndef VK_API_VERSION
#define VK_API_VERSION 11
#endif

#include "vkFFT.h"

struct NovaVkfftPlan {
  VkFFTApplication app = VKFFT_ZERO_INIT;
  VkFFTConfiguration cfg = VKFFT_ZERO_INIT;
  VkFFTLaunchParams launch = VKFFT_ZERO_INIT;

  VkPhysicalDevice physical = VK_NULL_HANDLE;
  VkDevice device = VK_NULL_HANDLE;
  VkQueue queue = VK_NULL_HANDLE;
  VkCommandPool command_pool = VK_NULL_HANDLE;
  VkFence fence = VK_NULL_HANDLE;

  VkBuffer buffer = VK_NULL_HANDLE;
  pfUINT buffer_size = 0;

  VkCommandBuffer cmd_buf = VK_NULL_HANDLE;
};

extern "C" {

NovaVkfftPlan* novasdr_vkfft_create_plan(uint64_t physical_device_raw,
                                        uint64_t device_raw,
                                        uint64_t queue_raw,
                                        uint64_t command_pool_raw,
                                        uint64_t fence_raw,
                                        uint64_t buffer_raw,
                                        uint64_t buffer_size_bytes,
                                        uint32_t fft_size,
                                        int* out_result) {
  if (out_result) {
    *out_result = (int)VKFFT_SUCCESS;
  }

  auto* p = new (std::nothrow) NovaVkfftPlan();
  if (!p) {
    if (out_result) {
      *out_result = (int)VKFFT_ERROR_MALLOC_FAILED;
    }
    return nullptr;
  }

  p->physical = (VkPhysicalDevice)(uintptr_t)physical_device_raw;
  p->device = (VkDevice)(uintptr_t)device_raw;
  p->queue = (VkQueue)(uintptr_t)queue_raw;
  p->command_pool = (VkCommandPool)(uintptr_t)command_pool_raw;
  p->fence = (VkFence)(uintptr_t)fence_raw;
  p->buffer = (VkBuffer)(uintptr_t)buffer_raw;
  p->buffer_size = (pfUINT)buffer_size_bytes;

  p->cfg.FFTdim = 1;
  p->cfg.size[0] = (pfUINT)fft_size;
  p->cfg.size[1] = 1;
  p->cfg.size[2] = 1;
  p->cfg.size[3] = 1;

  p->cfg.physicalDevice = &p->physical;
  p->cfg.device = &p->device;
  p->cfg.queue = &p->queue;
  p->cfg.commandPool = &p->command_pool;
  p->cfg.fence = &p->fence;

  p->cfg.bufferNum = 1;
  p->cfg.bufferSize = &p->buffer_size;
  p->cfg.buffer = &p->buffer;

  VkFFTResult res = initializeVkFFT(&p->app, p->cfg);
  if (res != VKFFT_SUCCESS) {
    if (out_result) {
      *out_result = (int)res;
    }
    deleteVkFFT(&p->app);
    delete p;
    return nullptr;
  }

  return p;
}

int novasdr_vkfft_record_forward(NovaVkfftPlan* plan, uint64_t command_buffer_raw) {
  if (!plan) {
    return (int)VKFFT_ERROR_PLAN_NOT_INITIALIZED;
  }

  plan->cmd_buf = (VkCommandBuffer)(uintptr_t)command_buffer_raw;

  plan->launch.commandBuffer = &plan->cmd_buf;
  plan->launch.buffer = &plan->buffer;
  plan->launch.bufferOffset = 0;

  VkFFTResult res = VkFFTAppend(&plan->app, 0, &plan->launch);
  return (int)res;
}

const char* novasdr_vkfft_error_string(int code) {
  return getVkFFTErrorString((VkFFTResult)code);
}

void novasdr_vkfft_destroy_plan(NovaVkfftPlan* plan) {
  if (!plan) {
    return;
  }
  deleteVkFFT(&plan->app);
  delete plan;
}

} // extern "C"
