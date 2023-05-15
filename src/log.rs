use std::{fs::{OpenOptions, File}, io::Write};
use chrono::prelude::*;

#[derive(Default)]
pub struct Log {
    pub log_file: Option<File>,
    pub log_text: String,
}

pub enum LogType {
    Info,
    Warn,
    Error,
}

impl Log {
    pub fn init_log(&mut self)
    {
        match OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open("Launch.log") {
                Ok(file) => self.log_file = Some(file),
                Err(e) => self.add_to_log(LogType::Error, format!("Failed to create log file! {}", e)),
            }
    }

    pub fn add_to_log(&mut self, log_type: LogType, log_data: String)
    {
        let datetime = Local::now();
        let timestamp_str = datetime.format("%Y-%m-%d %H:%M").to_string();
    
        let new_text: String;

        match log_type {
            LogType::Info => new_text = format!("[INFO] [{}] {}\n", timestamp_str, log_data),
            LogType::Warn => new_text = format!("[WARN] [{}] {}\n", timestamp_str, log_data),
            LogType::Error => new_text = format!("[ERROR] [{}] {}\n", timestamp_str, log_data),
        }

        if self.log_file.is_some() {
            self.log_file.as_mut().unwrap().write(&new_text.as_bytes()).unwrap_or_default();
        }

        self.log_text += &new_text;
    }
}