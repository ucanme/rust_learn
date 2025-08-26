use log4rs::config::{Appender, Config, Logger, Root};
use log4rs::append::console::ConsoleAppender;
use log4rs::append::file::FileAppender;
use log4rs::encode::pattern::PatternEncoder;
use log::LevelFilter;

pub(crate) fn init_config() {
    log4rs::init_file("log4rs.yaml", Default::default()).unwrap();
}