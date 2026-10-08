use dt_cli::{
    Runtime, credentials::SystemStore, login::SystemBrowser, output, pagination, profile,
};
#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = dt_cli::release::launch_managed() {
        let (value, code) = output::envelope_result("launcher", None, Err(error));
        println!("{}", output::render(&value, false));
        std::process::exit(code.into());
    }
    let args = std::env::args().collect();
    let config =
        profile::config_dir().and_then(|root| profile::environments().map(|envs| (root, envs)));
    let (root, environments) = match config {
        Ok(v) => v,
        Err(e) => {
            let (v, c) = output::envelope_result("cli", None, Err(e));
            println!("{}", output::render(&v, false));
            std::process::exit(c.into());
        }
    };
    let rt = Runtime {
        root,
        environments,
        store: Box::new(SystemStore),
        browser: Box::new(SystemBrowser),
        interactive: dt_cli::login::terminal(),
        aggregate_budget: std::time::Duration::from_secs(pagination::AGGREGATE_SECONDS),
    };
    let (v, exit, table) = dt_cli::execute(&rt, args).await;
    println!("{}", output::render(&v, table));
    std::process::exit(exit.into());
}
