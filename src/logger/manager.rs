use log::*;
use std::io::Write;

use crate::cli;
use chrono;
use env_logger::fmt::{Color, Style, StyledValue};

fn colored_level<'a>(style: &'a mut Style, level: Level) -> StyledValue<'a, &'static str> {
    match level {
        Level::Trace => style.set_color(Color::Magenta).value("TRACE"),
        Level::Debug => style.set_color(Color::Blue).value("DEBUG"),
        Level::Info => style.set_color(Color::Green).value("INFO "),
        Level::Warn => style.set_color(Color::Yellow).value("WARN "),
        Level::Error => style.set_color(Color::Red).value("ERROR"),
    }
}

pub fn init() {
    let default_filter = if cli::manager::is_verbose() {
        "debug"
    } else {
        "info"
    };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(default_filter))
        .format(|buf, record| {
            let mut style = buf.style();
            let level = colored_level(&mut style, record.level());
            let mut style = buf.style();
            let message = style.set_bold(true).value(record.args());
            writeln!(
                buf,
                "{} {} {}:{}: {}",
                level,
                chrono::Local::now().format("%H:%M:%S.%3f"),
                record.file().unwrap_or("unknown"),
                record.line().unwrap_or(0),
                message,
            )
        })
        .init();

    info!(
        "{}, version: {}-{}, build date: {}",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
        env!("VERGEN_GIT_SHA_SHORT"),
        env!("VERGEN_BUILD_DATE")
    );
    info!(
        "Starting at {}",
        chrono::Local::now().format("%Y-%m-%dT%H:%M:%S"),
    );
    debug!("Command line call: {}", cli::manager::command_line_string());
    debug!(
        "Command line input struct call: {:#?}",
        cli::manager::matches().args
    );
}
