use std::{path::PathBuf};
use chrono::prelude::*;

#[derive(Default)]
pub struct Log {
    pub log_file: PathBuf,
    pub log_text: String,
}

pub enum LogType {
    Info,
    Warn,
    Error,
}

impl Log {
    pub fn add_to_log(&mut self, log_type: LogType, log_data: String)
    {
        let datetime = Local::now();
        let timestamp_str = datetime.format("%Y-%m-%d %H:%M").to_string();
    
        match log_type {
            LogType::Info => self.log_text += &format!("[INFO] [{}] {}\n", timestamp_str, log_data),
            LogType::Warn => self.log_text += &format!("[WARN] [{}] {}\n", timestamp_str, log_data),
            LogType::Error => self.log_text += &format!("[ERROR] [{}] {}\n", timestamp_str, log_data),
        }
    }
}