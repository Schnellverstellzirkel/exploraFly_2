# Docs

Fixed-target reference for the native engine. Versions as fetched.

## Vulkan

- `vulkan/vk.xml`: Khronos Vulkan-Docs main. Machine-readable API and extension registry.
- `vulkan/extensions`: KHR and EXT specs used by the engine. Dynamic rendering, sync2, timeline semaphore, memory budget, host image copy, push descriptor, subgroup size control.
- `vulkan/guide`: Khronos Vulkan-Guide. Depth 1 clone.
- `vulkan/tutorial`: Overv VulkanTutorial. Depth 1 clone.

## NVIDIA

- `nvidia/VK_NV_*.adoc`: vendor specs. Low latency 2, device generated commands, mesh shader, diagnostic checkpoints, derivatives, coverage reduction, mixed samples, subgroup partitioned, scissor exclusive, clip space w scaling, sample mask override.

## AMD

- `amd/VK_AMD_*.adoc`: vendor specs. Texture gather bias, half float shader, negative viewport height, ballot, mixed attachment samples, early and late fragment tests.
- `amd/VulkanMemoryAllocator`: GPUOpen allocator and its docs. Depth 1 clone. Reference for the fixed memory plan.

## Linux

- `linux/man-pages`: kernel.org man-pages 6.12. Sections 2, 3, and 7 cover syscalls, threads, scheduling, and memory.

## Rust

- `rust/reference`: the Rust reference. Depth 1 clone.
- `rust/nomicon`: unsafe Rust rules. Depth 1 clone.
