use dt_cli::{Runtime, credentials::FileStore, login::SystemBrowser, output, pagination, profile};
#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = dt_cli::release::launch_managed() {
        let (value, code) = output::envelope_result("launcher", None, Err(error));
        println!("{}", output::render(&value, false));
        std::process::exit(code.into());
    }
    let args = std::env::args().collect();
    let config = config_root().and_then(|root| profile::environments().map(|envs| (root, envs)));
    let (root, environments) = match config {
        Ok(v) => v,
        Err(e) => {
            let (v, c) = output::envelope_result("cli", None, Err(e));
            println!("{}", output::render(&v, false));
            std::process::exit(c.into());
        }
    };
    let store = Box::new(FileStore::new(&root));
    let rt = Runtime {
        root,
        environments,
        store,
        browser: Box::new(SystemBrowser),
        interactive: dt_cli::login::terminal(),
        aggregate_budget: std::time::Duration::from_secs(pagination::AGGREGATE_SECONDS),
    };
    let (v, exit, table) = dt_cli::execute(&rt, args).await;
    println!("{}", output::render(&v, table));
    std::process::exit(exit.into());
}

// 只在本地开发构建中提供隔离原生子进程的根目录，正式制品不读取此变量。
fn config_root() -> output::Result<std::path::PathBuf> {
    #[cfg(feature = "local-dev")]
    if let Some(value) = std::env::var_os("DT_CLI_LOCAL_TEST_CONFIG") {
        let path = std::path::PathBuf::from(value);
        if !path.is_absolute() {
            return Err(output::invalid());
        }
        return Ok(path);
    }
    profile::config_dir()
}
