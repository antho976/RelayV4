mod app;
mod client;
mod editor;
mod icons;
mod pages;
mod smoke;
mod terminal;

fn main() -> glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("I/O runtime");
    app::run(runtime.handle().clone())
}
