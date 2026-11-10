use dt_cli::{Runtime, credentials::FileStore, login::SystemBrowser, output, profile};
#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = dt_cli::release::launch_managed() {
        let (value, code) = output::envelope_result("launcher", None, Err(error));
        println!("{}", output::render(&value, false));
        std::process::exit(code.into());
    }
    let args = std::env::args().collect();
    let config = config_root();
    let root = match config {
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
        store,
        browser: Box::new(SystemBrowser),
        interactive: dt_cli::login::terminal(),
        aggregate_budget: std::time::Duration::from_secs(30),
    };
    // 将根执行 Future 放到堆上，避免有界工作包的完整调用链占满原生主线程栈。
    let (v, exit, table) = Box::pin(dt_cli::execute(&rt, args)).await;
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
