use ::log::info;

mod log;
fn main() {
    log4rs::init_file("log.yml", Default::default()).unwrap();
    info!("这是一条 info 级别信息");
    // 针对特定 target（记录器）记录日志
    info!(target: "app::requests", "这是一个请求日志");
}