fn main() {
    let vkfft_enabled = std::env::var_os("CARGO_FEATURE_VKFFT").is_some();
    if !vkfft_enabled {
        return;
    }

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "linux" {
        println!("cargo:warning=vkfft is only supported on Linux targets (target_os={target_os})");
        panic!("vkfft is only supported on Linux targets");
    }

    let manifest_dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());

    fn vkfft_layout_ok(include_dir: &std::path::Path) -> bool {
        include_dir.join("vkFFT.h").is_file()
            && include_dir
                .join("vkFFT")
                .join("vkFFT_Structs")
                .join("vkFFT_Structs.h")
                .is_file()
    }

    fn find_vkfft_include_dir() -> Option<std::path::PathBuf> {
        if let Some(p) = std::env::var_os("VKFFT_INCLUDE_DIR").map(std::path::PathBuf::from) {
            if vkfft_layout_ok(&p) {
                return Some(p);
            }
        }

        // System packages (e.g. Debian/RPi: `libvkfft-dev`).
        let system_candidates = [
            "/usr/include/vkfft",
            "/usr/include/vkFFT",
            "/usr/include/VkFFT",
            "/usr/include",
            "/usr/local/include/vkfft",
            "/usr/local/include/vkFFT",
            "/usr/local/include/VkFFT",
            "/usr/local/include",
        ];
        for c in system_candidates {
            let p = std::path::PathBuf::from(c);
            if vkfft_layout_ok(&p) {
                return Some(p);
            }
        }

        None
    }

    let vkfft_include_dir = find_vkfft_include_dir();
    let Some(vkfft_include_dir) = vkfft_include_dir else {
        println!("cargo:warning=vkFFT.h not found.");
        println!(
            "cargo:warning=Install a system package (e.g. Debian/RPi: `apt-get install -y libvkfft-dev`)."
        );
        println!("cargo:warning=You can also set VKFFT_INCLUDE_DIR to the directory containing vkFFT.h and the vkFFT/ subdirectory.");
        panic!("vkfft requires VkFFT headers (vkFFT.h)");
    };

    fn find_glslang_include_dir() -> Option<std::path::PathBuf> {
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        if let Some(p) = std::env::var_os("GLSLANG_INCLUDE_DIR").map(std::path::PathBuf::from) {
            candidates.push(p);
        }

        // Common distro layouts:
        // - /usr/include/glslang/glslang_c_interface.h
        // - /usr/include/glslang/Include/glslang_c_interface.h
        // - /usr/local/include/...
        candidates.push(std::path::PathBuf::from("/usr/include/glslang"));
        candidates.push(std::path::PathBuf::from("/usr/include/glslang/Include"));
        candidates.push(std::path::PathBuf::from("/usr/local/include/glslang"));
        candidates.push(std::path::PathBuf::from(
            "/usr/local/include/glslang/Include",
        ));
        candidates.push(std::path::PathBuf::from("/usr/include"));
        candidates.push(std::path::PathBuf::from("/usr/local/include"));

        for base in candidates {
            if base.join("glslang_c_interface.h").exists() {
                return Some(base);
            }
            if base.join("Include").join("glslang_c_interface.h").exists() {
                return Some(base.join("Include"));
            }
        }
        None
    }

    let glslang_include_dir = find_glslang_include_dir();
    let Some(glslang_include_dir) = glslang_include_dir else {
        println!("cargo:warning=glslang_c_interface.h not found (set GLSLANG_INCLUDE_DIR or install glslang dev headers).");
        panic!("vkfft requires glslang headers (glslang_c_interface.h)");
    };

    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let wrapper = manifest_dir.join("src").join("dsp").join("vkfft_ffi.cpp");
    println!("cargo:rerun-if-changed={}", wrapper.display());

    fn library_search_paths() -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();

        // Prefer paths discovered via pkg-config if available.
        if let Ok(vulkan) = pkg_config::Config::new().probe("vulkan") {
            out.extend(vulkan.link_paths);
        }

        // Common multiarch locations.
        out.push(std::path::PathBuf::from("/usr/lib"));
        out.push(std::path::PathBuf::from("/usr/local/lib"));

        if let Ok(triple) = std::env::var("CARGO_CFG_TARGET_ARCH") {
            // Not perfect, but helps on Debian multiarch.
            out.push(std::path::PathBuf::from(format!(
                "/usr/lib/{triple}-linux-gnu"
            )));
        }

        // Deduplicate while preserving order.
        let mut seen = std::collections::HashSet::<std::path::PathBuf>::new();
        out.retain(|p| seen.insert(p.clone()));
        out
    }

    fn find_library_file(paths: &[std::path::PathBuf], name: &str) -> Option<std::path::PathBuf> {
        // Prefer the unversioned linker names.
        for dir in paths {
            for suffix in [".so", ".a"] {
                let candidate = dir.join(format!("lib{name}{suffix}"));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }

        // Some distros only ship versioned .so.<N> files in the non-dev package, without the
        // unversioned libfoo.so symlink. In that case, link by absolute path.
        for dir in paths {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else {
                        continue;
                    };
                    if file_name.starts_with(&format!("lib{name}.so.")) && path.is_file() {
                        return Some(path);
                    }
                }
            }
        }

        None
    }

    fn link_if_present(paths: &[std::path::PathBuf], name: &str) -> bool {
        let Some(path) = find_library_file(paths, name) else {
            println!("cargo:warning=vkfft: skipping missing library -l{name}");
            return false;
        };

        let is_unversioned_so = path.extension().and_then(|s| s.to_str()) == Some("so");
        let is_static_a = path.extension().and_then(|s| s.to_str()) == Some("a");
        if is_unversioned_so || is_static_a {
            println!("cargo:rustc-link-lib={name}");
            return true;
        }

        // Versioned shared object path: pass the full filename to the linker.
        println!("cargo:rustc-link-arg={}", path.display());
        true
    }

    fn add_pkg_config_search_paths(name: &str) -> bool {
        let Ok(lib) = pkg_config::Config::new().probe(name) else {
            return false;
        };

        for lib_path in lib.link_paths {
            println!("cargo:rustc-link-search=native={}", lib_path.display());
        }

        true
    }

    // VkFFT itself is header-only, but it depends on Vulkan + glslang for shader compilation.
    // We link to Vulkan and the glslang static/shared libs from the system toolchain.
    //
    // If you hit missing-library errors on your distro, install glslang development packages.
    // On Debian/Ubuntu this is typically: `apt-get install -y glslang-dev libvulkan-dev`.
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .file(&wrapper)
        .flag_if_supported("-std=c++11")
        .flag_if_supported("-fPIC")
        // Some environments export `CFLAGS/CXXFLAGS=-Werror` globally (common in CI/build roots).
        // VkFFT headers currently trigger a few warnings on newer compilers; they are upstream and
        // not actionable for NovaSDR. Ensure they don't fail the build.
        .flag_if_supported("-Wno-error")
        .flag_if_supported("-Wno-error=implicit-fallthrough")
        .flag_if_supported("-Wno-error=dangling-pointer")
        .flag_if_supported("-Wno-error=maybe-uninitialized")
        .flag_if_supported("-Wno-implicit-fallthrough")
        .flag_if_supported("-Wno-dangling-pointer")
        .flag_if_supported("-Wno-maybe-uninitialized")
        .include(&vkfft_include_dir)
        .include(&glslang_include_dir)
        .define("VKFFT_BACKEND", "0")
        .define("VK_API_VERSION", "11");

    // Ensure Vulkan headers are available.
    if let Ok(vulkan) = pkg_config::Config::new().probe("vulkan") {
        for p in vulkan.include_paths {
            build.include(p);
        }
        for lib_path in vulkan.link_paths {
            println!("cargo:rustc-link-search=native={}", lib_path.display());
        }
    } else {
        println!(
            "cargo:warning=pkg-config could not find vulkan; relying on default include/lib paths"
        );
        println!("cargo:rustc-link-lib=vulkan");
    }

    // Prefer glslang via pkg-config for discovery, but do not trust its `libs` list for ordering:
    // Debian arm64 commonly ships `libglslang.a` and may emit SPIRV-Tools too early, which breaks
    // static linking. We always link glslang libs explicitly and handle SPIRV-Tools afterwards.
    let _ = add_pkg_config_search_paths("glslang");

    {
        let paths = library_search_paths();
        for p in &paths {
            println!("cargo:rustc-link-search=native={}", p.display());
        }

        for lib in [
            "glslang",
            "MachineIndependent",
            "GenericCodeGen",
            "OSDependent",
            "SPVRemapper",
            "SPIRV",
            // Some distros don't ship these; link only if present.
            "OGLCompiler",
        ] {
            link_if_present(&paths, lib);
        }
    }

    // SPIRV-Tools: required when glslang is static (common on Debian arm64) and when VkFFT uses
    // glslang's SPIR-V tooling paths.
    //
    // We deliberately emit these *after* glslang to satisfy static link ordering.
    // SPIRV-Tools: glslang can be a static archive that depends on SPIRV-Tools. On some distros
    // SPIRV-Tools is shipped as multiple component archives, and `opt` depends on `tools`.
    // Ensure `SPIRV-Tools` comes *after* `SPIRV-Tools-opt` to satisfy static link resolution.
    let _ = add_pkg_config_search_paths("SPIRV-Tools");
    let _ = add_pkg_config_search_paths("SPIRV-Tools-opt");

    {
        let paths = library_search_paths();
        for p in &paths {
            println!("cargo:rustc-link-search=native={}", p.display());
        }

        for lib in [
            // Order matters for static libs: `SPIRV-Tools-opt` depends on symbols in `SPIRV-Tools`.
            "SPIRV-Tools-opt",
            "SPIRV-Tools-link",
            "SPIRV-Tools-reduce",
            "SPIRV-Tools-lint",
            "SPIRV-Tools-diff",
            "SPIRV-Tools",
        ] {
            link_if_present(&paths, lib);
        }
    }

    build.compile("novasdr_vkfft");

    // Make the OUT_DIR visible for debugging.
    println!("cargo:warning=vkfft build output: {}", out_dir.display());
}
