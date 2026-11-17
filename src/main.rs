fn main() -> std::process::ExitCode {
    #[cfg(unix)]
    if let Some(result) = boothop::host::openrc::dispatch(&std::env::args_os().collect::<Vec<_>>())
    {
        return match result {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error:#}");
                std::process::ExitCode::FAILURE
            }
        };
    }
    #[cfg(windows)]
    if let Some(result) =
        boothop::host::windows_escalation::dispatch(&std::env::args_os().collect::<Vec<_>>())
    {
        return match result {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error:#}");
                std::process::ExitCode::FAILURE
            }
        };
    }
    let mut system = boothop::host::Host::new();
    let output = boothop::cli::run(std::env::args_os(), &mut system);
    print!("{}", output.stdout);
    eprint!("{}", output.stderr);
    std::process::ExitCode::from(output.code)
}
