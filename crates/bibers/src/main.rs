use std::process::ExitCode;

/// mimalloc instead of the platform allocator: less fragmentation (peak RSS
/// 15% lower on large inputs) and fewer cycles per allocation.
#[cfg(not(feature = "dhat-heap"))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// `--features dhat-heap`: dhat's profiling allocator.
#[cfg(feature = "dhat-heap")]
#[global_allocator]
static GLOBAL: dhat::Alloc = dhat::Alloc;

fn main() -> ExitCode {
    #[cfg(feature = "dhat-heap")]
    let _profiler = dhat::Profiler::new_heap();
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let env = bibers_core::env::Env::from_process();
    ExitCode::from(bibers::run::main_then_exit(&args, &env))
}
