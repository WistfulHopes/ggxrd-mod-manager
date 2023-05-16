#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{path::{PathBuf, Path}, fs::{self}, ffi::OsStr, io::Cursor, process::{Command, exit}, sync::Mutex};
use lazy_static::lazy_static;
use egui::{self, text::LayoutJob, TextFormat, FontId, FontFamily, Color32, Ui, RichText};
use egui_dnd::{DragDropUi, utils::shift_vec};
use ini::{Ini, EscapePolicy};
use log::{Log, LogType};
use mod_data::ModData;
use self_update::cargo_crate_version;
use single_instance::SingleInstance;
use steamlocate::SteamDir;
use sysinfo::{System, SystemExt};
use tempfile::TempDir;
use winreg::{RegKey, enums::{RegDisposition::{REG_CREATED_NEW_KEY, REG_OPENED_EXISTING_KEY}, HKEY_CURRENT_USER}};

mod mod_data;
mod log;
mod helpers;
mod download;

lazy_static! {
    static ref CONFIG: Mutex<ConfigState> = Mutex::new(ConfigState::default());
    static ref WINDOW: Mutex<WindowState> = Mutex::new(WindowState::default());
}

pub(crate) fn load_icon() -> eframe::IconData {
	let (icon_rgba, icon_width, icon_height) = {
		let icon = include_bytes!("../assets/icon.png");
		let image = image::load_from_memory(icon)
			.expect("Failed to open icon path")
			.into_rgba8();
		let (width, height) = image.dimensions();
		let rgba = image.into_raw();
		(rgba, width, height)
	};
	
	eframe::IconData {
		rgba: icon_rgba,
		width: icon_width,
		height: icon_height,
	}
}

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        initial_window_size: Some(egui::vec2(1280.0, 720.0)),
        icon_data: Some(load_icon()),
        ..Default::default()
    };
    let mut manager: Box<ManagerState> = Box::<ManagerState>::default();

    manager.console_visible = true;
    
    let modmanager_instance = SingleInstance::new("e3ff4d30-0d65-45c2-8afd-8bff90d8569a").unwrap();
    let is_running: bool = !modmanager_instance.is_single();

    let args: Vec<String> = std::env::args().collect();
    if args.len() > 2 && args[1] == "-download" {
        manager.update_mods();
        if is_running {
            manager.append_log();
        }
        else {
            manager.init_log();
        }
        match prepare_download(args[2].to_owned()) {
            Ok((path, _tempdir)) => {
                let mut config: std::sync::MutexGuard<ConfigState> = CONFIG.lock().unwrap();
                manager.install_mod(path, &mut config);
            }
            Err(e) => manager.log.add_to_log(LogType::Error, format!("Could not download mod! {}", e))
        }

        manager.update_mods();
        let mut config: std::sync::MutexGuard<ConfigState> = CONFIG.lock().unwrap();
        manager.set_mod_order_config(&mut config);
        manager.write_config(&mut config);

        return Ok(())
    }
    else if is_running {
        return Ok(())
    }

    manager.init_log();
    manager.init_update();
    manager.init_steam();
    match manager.init_registry() {
        Ok(_) => manager.log.add_to_log(LogType::Info, "Successfully changed registry!".to_owned()),
        Err(e) => manager.log.add_to_log(LogType::Info, format!("Failed to change registry! {}", e)),
    }

    eframe::run_native(
        "GUILTY GEAR Xrd Mod Manager",
        options,
        Box::new(|_cc| manager),
    )
}

fn prepare_download (line: String) -> Result<(PathBuf, TempDir), Box<dyn std::error::Error>> {
    let new_line = line.replace("xrdmodman:", "");
    let parts: Vec<&str> = new_line.split(",").collect();
    Ok(download::download_mod(parts[0].to_owned())?)
}

#[derive(Default)]
struct ManagerState {
    dnd: DragDropUi,
    game_path: PathBuf,
    mods_path: PathBuf,
    mod_edit: ModData,
    mod_datas: Vec<ModData>,
    selected_mod: ModData,
    log: Log,
    console_visible: bool,
}

#[derive(Default)]
struct ConfigState {
    config: Ini,
}

#[derive(Default)]
struct WindowState {
    about_open: bool,
    create_open: bool,
    edit_open: bool,
    remove_open: bool,
}

impl ManagerState {
    fn init_registry(&mut self) -> std::io::Result<()> {
        let hkcr = RegKey::predef(HKEY_CURRENT_USER);
        let path = Path::new("Software").join("Classes").join("xrdmodman");
        let (key, disp) = hkcr.create_subkey(&path)?;
        
        match disp {
            REG_CREATED_NEW_KEY => self.log.add_to_log(LogType::Info, "Created url registry key for xrdmodman!".to_owned()),
            REG_OPENED_EXISTING_KEY => self.log.add_to_log(LogType::Info, "Opened url registry key for xrdmodman!".to_owned()),
        }

        key.set_value("", &"URL:xrdmodman")?;
        key.set_value("URL Protocol", &"")?;

        let new_path = path.join("shell").join("open").join("command");

        let (new_key, new_disp) = hkcr.create_subkey(&new_path)?;
        
        match new_disp {
            REG_CREATED_NEW_KEY => self.log.add_to_log(LogType::Info, "Created command registry key for xrdmodman!".to_owned()),
            REG_OPENED_EXISTING_KEY => self.log.add_to_log(LogType::Info, "Opened command registry key for xrdmodman!".to_owned()),
        }

        let exe_path = std::env::current_exe()?;
        let command = r#" -download "%1""#;

        new_key.set_value("", &(r#"""#.to_owned() + &exe_path.display().to_string() + r#"""# + command))
    }

    fn init_update(&mut self) {
        match helpers::update() {
            Ok(status) => {
                match status {
                    self_update::Status::UpToDate(_) => self.log.add_to_log(LogType::Info, "You are on the latest version!".to_owned()),
                    self_update::Status::Updated(_) => 
                    {
                        self.log.add_to_log(LogType::Info, "Update successful! Restarting...".to_owned());
                        Command::new("ggxrd-mod-manager.exe").spawn().unwrap();
                        exit(0)
                    }
                }
            }
            Err(e) => self.log.add_to_log(LogType::Error, format!("Update failed! {}", e)),
        }
    }
    
    fn mods_layout(&mut self, ui: &mut Ui) -> (bool, bool)
    {
        let mut config_needs_update = false;
        let mut edit_flag = false;
        let response = self.dnd.ui::<ModData>(ui, self.mod_datas.iter_mut(), |mod_data, ui, handle| {
            ui.horizontal(|ui| {
                if ui.checkbox(&mut mod_data.enabled, "").changed() {
                    update_mod_config(mod_data.name.clone(), mod_data);
                    config_needs_update = true;
                };
                let response = ui.selectable_label(true, &mod_data.name);
                if response.clicked() {
                    self.selected_mod = mod_data.clone();
                }
                let popup_id = ui.make_persistent_id(format!("right_click_menu_{}", mod_data.name));
                if response.secondary_clicked() {
                    self.selected_mod = mod_data.clone();
                    ui.memory_mut(|mem|{
                        mem.toggle_popup(popup_id)
                    });
                }
                egui::popup::popup_below_widget(ui, popup_id, &response, |ui| {
                    let mut window = WINDOW.lock().unwrap();
                    ui.set_min_width(150.);
                    if ui.button("Open containing folder").clicked() {
                        open::that(mod_data.path.clone()).unwrap_or_default();
                    }
                    if ui.button("Edit mod").clicked() {
                        window.edit_open = true;
                        edit_flag = true;
                    }
                    if ui.button("Remove mod").clicked() {
                        window.remove_open = true;
                    }
                });
                handle.ui(ui, mod_data, |ui| {
                    ui.separator();
                })
            });
        });
        if let Some(completed) = response.completed {
            shift_vec(completed.from, completed.to, &mut self.mod_datas);
            for (i, data) in self.mod_datas.iter_mut().enumerate() {
                data.order = i;
            }
            config_needs_update = true;
        }
        (config_needs_update, edit_flag)
    }
}

fn init_mod_config(mod_name: String, data: &mut ModData, config: &mut ConfigState)
{
    let section = config.config.section(Some("Mods"));
    match section
    {
        Some(section) => {
            let entry: Option<&str> = section.get(&mod_name);
            match entry {
                Some(entry) => {
                    match entry {
                        "True" => data.enabled = true,
                        _ => data.enabled = false,
                    }
                }
                None => {
                    config.config.with_section(Some("Mods")).set(&mod_name, "True");
                }
            }
        }
        None => {
            config.config.with_section(Some("Mods")).set(&mod_name, "True");
        }
    }
}

fn update_mod_config(mod_name: String, data: &mut ModData)
{
    let mut config = CONFIG.lock().unwrap();
    match data.enabled {
        true => {
            config.config.with_section(Some("Mods")).set(&mod_name, "True");
        }
        false => {
            config.config.with_section(Some("Mods")).set(&mod_name, "False");
        }
    }
}

fn remove_mod_config(mod_name: String)
{
    let mut config = CONFIG.lock().unwrap();
    config.config.with_section(Some("Mods")).delete(&mod_name);
}

impl ManagerState {
    fn create_config(&mut self, config: &mut ConfigState)
    {
        let mut ini = Ini::new();
        ini.with_section(Some("General"))
            .set("ConsoleVisible", "True");
        self.write_config(config)
    }

    fn write_config(&mut self, config: &mut ConfigState)
    {
        let mut exe_path = std::env::current_exe().unwrap();
        exe_path.pop();
        let ini_path = exe_path.join("config.ini");
        match config.config.write_to_file(ini_path)
        {
            Ok(_) => (),
            Err(e) => self.log.add_to_log(LogType::Error, format!("Could not create config ini! {}", e))
        }
    }

    fn set_mod_order_config(&mut self, config: &mut ConfigState)
    {
        config.config.delete(Some("Mods"));
        for mod_data in &self.mod_datas {
            let enabled = match mod_data.enabled {
                true => "True",
                false => "False",
            };
            config.config.with_section(Some("Mods"))
                .set(mod_data.name.clone(), enabled);
        }
        self.write_config(config)
    }

    fn init_steam(&mut self)
    {
        let steamdir: Option<SteamDir> = SteamDir::locate();
        match steamdir {
            Some(mut dir) => {
                match dir.app(&520440)
                {
                    Some(app) => {
                        self.game_path = app.path.clone();
                        self.log.add_to_log(LogType::Info, format!("Guilty Gear Xrd Rev 2 located at {}.", app.path.display()))
                    },
                    None => self.log.add_to_log(LogType::Error, "Could not locate Guilty Gear Xrd Rev 2! Make sure you have it installed.".to_owned())
                }
            },
            None => self.log.add_to_log(LogType::Error, "Could not locate Steam!".to_owned())
        }
    }

    fn init_config(&mut self)
    {
        let mut config = CONFIG.lock().unwrap();
        let mut exe_path = std::env::current_exe().unwrap();
        exe_path.pop();
        let ini_path = exe_path.join("config.ini");
        if ini_path.exists() {
            let ini = Ini::load_from_file_noescape(ini_path);
            match ini {
                Ok(ini) => config.config = ini,
                Err(_) => self.create_config(&mut config),
            }
        }
        else 
        {
            self.create_config(&mut config)
        } 
    }

    fn update_mods(&mut self)
    {
        self.init_config();
        self.mod_datas.clear();
        let mut dir = std::env::current_exe().unwrap();
        dir.pop();
        self.mods_path = Path::join(&dir, "Mods");
        match fs::create_dir(&self.mods_path)
        {
            Ok(_) => (),
            Err(ref e) => {
                if e.kind() == std::io::ErrorKind::AlreadyExists {()}
                else {
                    self.log.add_to_log(LogType::Error, format!("Could not create Mods directory! {}", e))
                }
            }
        }
        let mut config: std::sync::MutexGuard<ConfigState> = CONFIG.lock().unwrap();
        let mod_section = config.config.section(Some("Mods"));
        let mut config_requires_update = false;
        match mod_section {
            Some(mod_section) => {
                for mod_entry in mod_section.iter() {
                    let path = Path::join(&self.mods_path, mod_entry.0).join("mod.ini");
                    if path.exists()
                    {
                        let mut mod_data = ModData::new();
                        let ini: Result<Ini, ini::Error> = Ini::load_from_file_noescape(&path);
                        match ini {
                            Ok(file) => {
                                let desc_section: Option<&ini::Properties> = file.section(Some("Description"));
                                match desc_section {
                                    Some(desc) => {
                                        let mod_name: Option<&str> = desc.get("Name");
                                        match mod_name {
                                            Some(name) => mod_data.name = name.to_owned(),
                                            None => {
                                                self.log.add_to_log(LogType::Warn, format!("The mod ini at path {} doesn't have a name in the desciption section! Ignoring mod.", path.display()));
                                                continue
                                            }
                                        }
                                        let mod_author = desc.get("Author");
                                        match mod_author {
                                            Some(author) => mod_data.author = author.to_owned(),
                                            None => ()
                                        }
                                        let mod_version = desc.get("Version");
                                        match mod_version {
                                            Some(version) => mod_data.version = version.to_owned(),
                                            None => ()
                                        }
                                        let mod_category = desc.get("Category");
                                        match mod_category {
                                            Some(category) => mod_data.category = category.to_owned(),
                                            None => ()
                                        }
                                        let mod_description = desc.get("Description");
                                        match mod_description {
                                            Some(description) => mod_data.description = description.to_owned(),
                                            None => ()
                                        }
                                        let mod_page = desc.get("Page");
                                        match mod_page {
                                            Some(page) => mod_data.page = page.to_owned(),
                                            None => ()
                                        }

                                        match file.section(Some("Scripts"))
                                        {
                                            Some(section) => {
                                                for script in section.get_all("ScriptPackage")
                                                {
                                                    mod_data.scripts.push(script.to_owned());
                                                }
                                            }
                                            None => (),
                                        }

                                        mod_data.path = Path::join(&self.mods_path, &mod_name.unwrap());
                                        mod_data.enabled = match mod_entry.1 {
                                            "True" => true,
                                            "False" => false,
                                            _ => true,
                                        };
                                        mod_data.order = self.mod_datas.len();
                                        self.mod_datas.push(mod_data);
                                    },
                                    None => {
                                        self.log.add_to_log(LogType::Error, format!("The mod ini at path {} doesn't have a description section! Ignoring mod.", path.display()));
                                        config_requires_update = true;
                                        continue
                                    }
                                }
                            },
                            Err(_) => {
                                self.log.add_to_log(LogType::Error, format!("Ini at path {} does not exist! Ignoring mod.", path.display()));
                                config_requires_update = true;
                                continue
                            }
                        }
                    }
                    else {
                        self.log.add_to_log(LogType::Error, format!("Path {} does not exist! Ignoring mod.", path.display()));
                        config_requires_update = true;
                    }
                }
            }
            None => (),
        }
        for mod_data in &mut self.mod_datas {
            init_mod_config(mod_data.name.clone(), mod_data, &mut config);
        }
        if config_requires_update {
            self.set_mod_order_config(&mut config)
        }
    }

    fn init_log(&mut self) {
        self.log.init_log();
        self.log.add_to_log(LogType::Info, "Launched GUILTY GEAR Xrd Mod Manager.".to_owned());
    }

    fn append_log(&mut self) {
        self.log.append_log();
        self.log.add_to_log(LogType::Info, "Another instance of the mod manager was opened!".to_owned());
    }

    fn init_mod(&mut self, name: String, config: &mut ConfigState)
    {
        for mod_data in &self.mod_datas {
            if name == mod_data.name {
                return
            }
        }

        let path = Path::join(&self.mods_path, &name).join("mod.ini");
        if path.exists()
        {
            let mut mod_data: ModData = ModData::new();
            let ini: Result<Ini, ini::Error> = Ini::load_from_file_noescape(&path);
            match ini {
                Ok(file) => {
                    let desc_section: Option<&ini::Properties> = file.section(Some("Description"));
                    match desc_section {
                        Some(desc) => {
                            let mod_name = desc.get("Name");
                            match mod_name {
                                Some(name) => mod_data.name = name.to_owned(),
                                None => {
                                    self.log.add_to_log(LogType::Warn, format!("The mod ini at path {} doesn't have a name in the desciption section! Ignoring mod.", path.display()));
                                }
                            }
                            let mod_author = desc.get("Author");
                            match mod_author {
                                Some(author) => mod_data.author = author.to_owned(),
                                None => ()
                            }
                            let mod_version = desc.get("Version");
                            match mod_version {
                                Some(version) => mod_data.version = version.to_owned(),
                                None => ()
                            }
                            let mod_category = desc.get("Category");
                            match mod_category {
                                Some(category) => mod_data.category = category.to_owned(),
                                None => ()
                            }
                            let mod_description = desc.get("Description");
                            match mod_description {
                                Some(description) => mod_data.description = description.to_owned(),
                                None => ()
                            }
                            let mod_page = desc.get("Page");
                            match mod_page {
                                Some(page) => mod_data.page = page.to_owned(),
                                None => ()
                            }
                            
                            match file.section(Some("Scripts"))
                            {
                                Some(section) => {
                                    for script in section.get_all("ScriptPackage")
                                    {
                                        mod_data.scripts.push(script.to_owned());
                                    }
                                }
                                None => (),
                            }
    
                            mod_data.path = Path::join(&self.mods_path, &name);
                            init_mod_config(mod_name.unwrap().to_owned(), &mut mod_data, config);
                            self.write_config(config);
                            self.mod_datas.push(mod_data);
                        },
                        None => {
                            mod_data.name = name.clone();
                            mod_data.path = Path::join(&self.mods_path, &name);
                            mod_data.write_data().unwrap_or_default();
                            init_mod_config(name, &mut mod_data, config);
                            self.write_config(config);
                            self.mod_datas.push(mod_data);
                            self.log.add_to_log(LogType::Warn, format!("The mod ini at path {} doesn't have a description section! Created one automatically.", &path.display()));
                        }
                    }
                },
                Err(_) => {
                    mod_data.name = name.clone();
                    mod_data.path = Path::join(&self.mods_path, &name);
                    mod_data.write_data().unwrap_or_default();
                    init_mod_config(name, &mut mod_data, config);
                    self.write_config(config);
                    self.mod_datas.push(mod_data);
                    self.log.add_to_log(LogType::Warn, format!("No mod ini at path {}! Created one automatically.", &path.display()));
                }
            }
        }
        else {
            let mut mod_data: ModData = ModData::new();
            mod_data.name = name.clone();
            mod_data.path = Path::join(&self.mods_path, &name);
            mod_data.write_data().unwrap_or_default();
            init_mod_config(name, &mut mod_data, config);
            self.write_config(config);
            self.mod_datas.push(mod_data);
            self.log.add_to_log(LogType::Warn, format!("No mod ini at path {}! Created one automatically.", &path.display()));
        }
    }

    fn install_mod(&mut self, path: PathBuf, config: &mut ConfigState)
    {
        let file_type: i32 = match path.extension().and_then(OsStr::to_str)
        {
            Some("zip") => 0,
            Some("7z") => 1,
            Some("rar") => 2,
            _ => 3,
        };
        let file_stem = match path.file_stem() {
            Some(file_stem) => file_stem,
            None => {
                self.log.add_to_log(LogType::Error, "File has no name!".to_owned());
                return
            }
        };
        match file_type {
            0 => {
                match std::fs::read(&path) {
                    Ok(bytes) => {
                        match zip_extract::extract(Cursor::new(bytes), 
                            &Path::join(&self.mods_path, file_stem), true)
                        {
                            Ok(_) => self.init_mod(file_stem.to_str().unwrap().to_owned(), config),
                            Err(e) => self.log.add_to_log(LogType::Error, format!("Could not extract archive! {}", e))
                        }
                    }
                    Err(e) => {
                        self.log.add_to_log(LogType::Error, format!("Could not read archive! {}", e))
                    }
                }
            }
            1 => {
                match sevenz_rust::decompress_file(&path, Path::join(&self.mods_path, file_stem))
                {
                    Ok(_) => self.init_mod(file_stem.to_str().unwrap().to_owned(), config),
                    Err(e) => self.log.add_to_log(LogType::Error, format!("Could not extract archive! {}", e))
                }        
            }
            2 => {
                match unrar::Archive::new(&path) {
                    Ok(archive) => 
                    {
                        match archive.extract_to(Path::join(&self.mods_path, file_stem))
                        {
                            Ok(mut archive) => {
                                match archive.process() {
                                    Ok(_) => self.init_mod(file_stem.to_str().unwrap().to_owned(), config),
                                    Err(e) => self.log.add_to_log(LogType::Error, format!("Could not extract archive! {}", e))
                                }
                            },
                            Err(e) => self.log.add_to_log(LogType::Error, format!("Could not extract archive! {}", e))
                        }        
                    }
                    Err(e) => {
                        self.log.add_to_log(LogType::Error, format!("Could not read archive! {}", e))
                    }
                }
            }
            _ => {
                self.log.add_to_log(LogType::Error, "Invalid file extension!".to_string())
            }
        }
    }

    fn file_menu(&mut self, ui: &mut Ui, config: &mut ConfigState)
    {
        if ui.button("Install Mod").clicked() {
            if let Some(path) = rfd::FileDialog::new()
            .add_filter("All supported archives", &["zip", "rar", "7z"])
            .add_filter("ZIP archive", &["zip"])
            .add_filter("7Z archive", &["7z"])
            .add_filter("RAR archive", &["rar"])
            .pick_file() {
                self.install_mod(path, config)
            };
            ui.close_menu();
        }
        let mut window = WINDOW.lock().unwrap();
        if ui.button("Create Mod").clicked() {
            window.create_open = true;
            ui.close_menu();
        }
        if ui.button("Locate Mod").clicked() {
            if let Some(path) = rfd::FileDialog::new()
            .add_filter("INI file", &["ini"])
            .pick_file() {
                let mut name = path.clone();
                name.pop();
                self.init_mod(name.display().to_string(), config)
            }
            ui.close_menu()
        }
    }

    fn settings_menu(&mut self, ui: &mut Ui)
    {
        if ui.checkbox(&mut self.console_visible, "Show Console").changed() {
            ui.close_menu();
        }
    }

    fn setup_mods_and_play(&mut self)
    {
        let ini_path = Path::join(&self.game_path, "REDGame").join("Config").join("DefaultEngine.ini");
        let ini: Result<Ini, ini::Error> = Ini::load_from_file_noescape(&ini_path);
        match ini {
            Ok(mut ini) => 
            {
                match ini.section_mut(Some("Engine.ScriptPackages"))
                {
                    Some(section) => {
                        for _ in section.remove_all("+NativePackages") {}
                        section.append("+NativePackages", "REDGame");
                        match ini.write_to_file_policy(&ini_path, EscapePolicy::Nothing) {
                            Ok(_) => (),
                            Err(e) => self.log.add_to_log(LogType::Error, format!("Could not write to DefaultEngine.ini! {}", e)),
                        }    
                    }
                    None => self.log.add_to_log(LogType::Error, "Could not find Engine.ScriptPackages in DefaultEngine.ini! Your game installation may be broken.".to_owned()),
                }
        }
            Err(e) => self.log.add_to_log(LogType::Error, format!("Could not read DefaultEngine.ini! {}", e)),
        }
        fs::remove_dir_all(Path::join(&self.game_path, "REDGame").join("CookedPCConsole").join("Mods")).unwrap_or_default();
        for mod_data in self.mod_datas.iter().rev() {
            if mod_data.enabled {
                let mut folder_string = "a".to_owned();
                let game_mods_path = Path::join(&self.game_path, "REDGame").join("CookedPCConsole").join("Mods");
                while Path::join(&game_mods_path, &folder_string).exists() {
                    let tmp_string = helpers::add1_str(&folder_string);
                    if folder_string != tmp_string {
                        folder_string = tmp_string;
                    }
                    else {
                        self.log.add_to_log(LogType::Error, format!("Could not copy mod {}! Too many mods installed.", &mod_data.name));
                        break;
                    }
                }
                match helpers::copy_recursively(&mod_data.path, Path::join(&game_mods_path, &folder_string).join(&mod_data.name))
                {
                    Ok(_) => (),
                    Err(e) => {
                        self.log.add_to_log(LogType::Error, format!("Could not copy mod {}! {}", &mod_data.name, e));
                        continue;
                    }
                }
                let ini_path: PathBuf = Path::join(&self.game_path, "REDGame").join("Config").join("DefaultEngine.ini");
                let ini: Result<Ini, ini::Error> = Ini::load_from_file_noescape(&ini_path);
                match ini {
                    Ok(mut ini) => {
                        for script in &mod_data.scripts {
                            match ini.section_mut(Some("Engine.ScriptPackages"))
                            {
                                Some(section) => {
                                    if section.get_all("+NativePackages").find(|x| x == script).is_none() {
                                        section.append("+NativePackages", script);
                                        self.log.add_to_log(LogType::Info, format!("Added script package {}!", script))
                                    }
                                }
                                None => self.log.add_to_log(LogType::Error, "Could not read find Engine.ScriptPackages in DefaultEngine.ini! Your game installation may be broken.".to_owned()),
                            }
                        }
                        match ini.write_to_file_policy(&ini_path, EscapePolicy::Nothing) {
                            Ok(_) => (),
                            Err(e) => self.log.add_to_log(LogType::Error, format!("Could not write to DefaultEngine.ini! {}", e)),
                        }
                    }
                    Err(e) => self.log.add_to_log(LogType::Error, format!("Could not read DefaultEngine.ini! {}", e)),
                }    
            }
        }
        self.log.add_to_log(LogType::Info, "Mods copied to game directory!".to_string());
        match open::that("steam://run/520440")
        {
            Ok(_) => self.log.add_to_log(LogType::Info, "Launching Guilty Gear Xrd Rev 2...".to_string()),
            Err(e) => self.log.add_to_log(LogType::Error, format!("Could not launch Guilty Gear Xrd Rev 2! {}", e)),
        }
    }
}

impl eframe::App for ManagerState {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame)
    {
        egui::TopBottomPanel::top("header_panel").show(ctx, |ui: &mut Ui| {
            ui.horizontal(|ui| {
                ui.menu_button("File", |ui| {
                    let mut config = CONFIG.lock().unwrap();
                    self.file_menu(ui, &mut config)
                });
                ui.menu_button("Settings", |ui| {
                    self.settings_menu(ui)
                });
                ui.menu_button("Help", |ui| {
                    help_menu(ui)
                });
                let mut visuals = ui.ctx().style().visuals.clone();
                visuals.light_dark_radio_buttons(ui);
                ui.ctx().set_visuals(visuals);
            });
        });
        
        if self.console_visible
        {
            let mut layouter = |ui: &Ui, string: &str, wrap_width: f32| {
                let mut job = LayoutJob::default();
                for line in string.lines() {
                    match line {
                        s if s.starts_with("[INFO]") =>
                        {
                            job.append(
                                line,
                                0.0,
                                TextFormat {
                                    font_id: FontId::new(14.0, FontFamily::Monospace),
                                    color: Color32::GREEN,
                                    ..Default::default()
                                },
                            );            
                        }
                        s if s.starts_with("[WARN]") =>
                        {
                            job.append(
                                line,
                                0.0,
                                TextFormat {
                                    font_id: FontId::new(14.0, FontFamily::Monospace),
                                    color: Color32::YELLOW,
                                    ..Default::default()
                                },
                            );            
                        }
                        s if s.starts_with("[ERROR]") =>
                        {
                            job.append(
                                line,
                                0.0,
                                TextFormat {
                                    font_id: FontId::new(14.0, FontFamily::Monospace),
                                    color: Color32::RED,
                                    ..Default::default()
                                },
                            );            
                        }
                        _ => 
                        {
                            job.append(
                                line,
                                0.0,
                                TextFormat {
                                    font_id: FontId::new(14.0, FontFamily::Monospace),
                                    color: Color32::WHITE,
                                    ..Default::default()
                                },
                            );            
    
                        }
                    }
                    job.append(
                        "\n",
                        0.0,
                        TextFormat {
                            font_id: FontId::new(14.0, FontFamily::Monospace),
                            color: Color32::WHITE,
                            ..Default::default()
                        },
                    );
                }
                job.wrap.max_width = wrap_width;
                ui.fonts(|f| f.layout_job(job))
            };
    
            egui::TopBottomPanel::bottom("console_panel")
            .max_height(300.)
            .resizable(true)
            .show(ctx, |ui: &mut Ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let mut log: &str = &self.log.log_text;
                    ui.add(
                    egui::TextEdit::multiline(&mut log)
                            .font(egui::TextStyle::Monospace) // for cursor height
                            .code_editor()
                            .desired_rows(10)
                            .lock_focus(true)
                            .desired_width(f32::INFINITY)
                            .layouter(&mut layouter),
                        );
                });
            });
        }
    
        egui::SidePanel::left("options_panel").show(ctx, |ui: &mut Ui| {
            ui.vertical(|ui| {
                // TODO implement browsing functionality, and swapping between it and managing
                /*if ui.small_button("🌐Browse Mods").clicked() {
    
                }
                if ui.small_button("📁Manage Mods").clicked() {
    
                }*/
                if ui.small_button("▶️Launch Game").clicked() {
                    let system = System::new_all();
                    if system.processes_by_exact_name("GuiltyGearXrd.exe").peekable().peek().is_some()
                    {
                        match Command::new("taskkill").args(["/f", "/im", "GuiltyGearXrd.exe"]).spawn()
                        {
                            Ok(_) => self.log.add_to_log(LogType::Info, "Stopping existing Guilty Gear Xrd process if it exists!".to_owned()),
                            Err(e) => self.log.add_to_log(LogType::Info, format!("Could not stop Guilty Gear Xrd process! {}", e)),
                        }    
                    }
                    self.setup_mods_and_play();
                }
            });
        });
    
        egui::SidePanel::right("details_panel")
            .max_width(f32::INFINITY)
            .min_width(280.)
            .show(ctx, |ui: &mut Ui| {
                ui.vertical(|ui| {
                    ui.label(format!("Author: {}", self.selected_mod.author));
                    ui.label(format!("Category: {}", self.selected_mod.category));
                    ui.label(format!("Description: {}", &self.selected_mod.description));
                    ui.label(format!("Version: {}", self.selected_mod.version));
                });
        });
    
        let mut config_needs_update = false;
        let mut edit_flag = false;
    
        egui::CentralPanel::default().show(ctx, |ui| {
            let mods_return_value = self.mods_layout(ui);
            config_needs_update = mods_return_value.0;
            edit_flag = mods_return_value.1;
        });
    
        let mut selected_index: usize = usize::MAX;
        for (index, data) in self.mod_datas.iter().enumerate() {
            if data.name == self.selected_mod.name {
                selected_index = index;
                break;
            }
        }
    
        if edit_flag {
            self.mod_edit = self.mod_datas[selected_index].clone();
        }
    
        if config_needs_update {
            let mut config = CONFIG.lock().unwrap();
            self.set_mod_order_config(&mut config);
            self.write_config(&mut config)
        }
    
        let mut window = WINDOW.lock().unwrap();
        let mut create_open: bool = window.create_open;
    
        egui::Window::new("Create Mod")
        .open(&mut create_open)
        .show(ctx, |ui| {
            ui.label(RichText::new("Fill out details about your mod.").size(18.));
    
            ui.label("Name");
            ui.text_edit_singleline(&mut self.mod_edit.name);
            ui.end_row();
    
            ui.label("Author");
            ui.text_edit_singleline(&mut self.mod_edit.author);
            ui.end_row();
    
            ui.label("Category");
            ui.text_edit_singleline(&mut self.mod_edit.category);
            ui.end_row();
    
            ui.label("Version");
            ui.text_edit_singleline(&mut self.mod_edit.version);
            ui.end_row();
    
            ui.label("Description");
            ui.text_edit_singleline(&mut self.mod_edit.description);
            ui.end_row();
    
            ui.label("UnrealScript Packages");
            for script in &mut self.mod_edit.scripts {
                ui.text_edit_singleline(script);
            }
            if ui.button("➕").clicked() {
                self.mod_edit.scripts.push("".to_owned());
            }
            if ui.button("➖").clicked() {
                self.mod_edit.scripts.pop();
            }
            ui.end_row();
    
            let ok_response = ui.button("OK");
            let error_id = ui.make_persistent_id("error");
    
            egui::popup::popup_below_widget(ui, error_id, &ok_response, |ui| {
                ui.set_min_width(150.);
                ui.label("Creation failed! Check log for more details.");
            });
    
            if ok_response.clicked() {
                if self.mod_edit.name.is_empty()
                {
                    ui.memory_mut(|mem|{
                        mem.toggle_popup(error_id);
                    });
                    self.log.add_to_log(LogType::Error, "You must give your mod a name!".to_owned());
                }
                else {
                    let mut duplicate_name = false;
                    for data in &self.mod_datas {
                        if data.name == self.mod_edit.name {
                            duplicate_name = true;
                        }
                    }
                    if duplicate_name {
                        ui.memory_mut(|mem|{
                            mem.toggle_popup(error_id);
                        });
                        self.log.add_to_log(LogType::Error, "A mod with that name already exists!".to_owned());
                    }
                    else {
                        self.mod_edit.order = self.mod_datas.len();
                        self.mod_edit.path = Path::join(&self.mods_path, &self.mod_edit.name);
                        let final_mod: ModData = self.mod_edit.clone();
                        match self.mod_edit.write_data() {
                            Ok(()) => {
                                let mut config = CONFIG.lock().unwrap();
                                self.log.add_to_log(LogType::Info, format!("Created mod {}!", &final_mod.name));
                                self.mod_datas.push(final_mod.clone());
                                self.set_mod_order_config(&mut config);
                                window.create_open = false;
                                open::that(final_mod.path.clone()).unwrap_or_default();
                            },
                            Err(e) => 
                            {
                                ui.memory_mut(|mem|{
                                    mem.toggle_popup(error_id);
                                });        
                                self.log.add_to_log(LogType::Error, format!("Could not create mod! {}", e))
                            }
                        }
                    }
                }
            }
        });
    
        window.create_open &= create_open;
    
        let mut edit_open: bool = window.edit_open;
    
        egui::Window::new("Edit Mod")
        .open(&mut edit_open)
        .show(ctx, |ui| {
            ui.label(RichText::new("Fill out details about your mod.").size(18.));
    
            ui.label("Name");
            ui.text_edit_singleline(&mut self.mod_edit.name);
            ui.end_row();
    
            ui.label("Author");
            ui.text_edit_singleline(&mut self.mod_edit.author);
            ui.end_row();
    
            ui.label("Category");
            ui.text_edit_singleline(&mut self.mod_edit.category);
            ui.end_row();
    
            ui.label("Version");
            ui.text_edit_singleline(&mut self.mod_edit.version);
            ui.end_row();
    
            ui.label("Description");
            ui.text_edit_singleline(&mut self.mod_edit.description);
            ui.end_row();
    
            ui.label("UnrealScript Packages");
            for script in &mut self.mod_edit.scripts {
                ui.text_edit_singleline(script);
            }
            if ui.button("➕").clicked() {
                self.mod_edit.scripts.push("".to_owned());
            }
            if ui.button("➖").clicked() {
                self.mod_edit.scripts.pop();
            }
            ui.end_row();
    
            let ok_response = ui.button("OK");
            let error_id = ui.make_persistent_id("error_edit");
    
            egui::popup::popup_below_widget(ui, error_id, &ok_response, |ui| {
                ui.set_min_width(150.);
                ui.label("Creation failed! Check log for more details.");
            });
    
            if ok_response.clicked() {
                if self.mod_edit.name.is_empty()
                {
                    ui.memory_mut(|mem|{
                        mem.toggle_popup(error_id);
                    });
                    self.log.add_to_log(LogType::Error, "You must give your mod a name!".to_owned());
                }
                else {
                    let mut duplicate_name = false;
                    for data in &self.mod_datas {
                        if data.name == self.mod_edit.name && data.name != self.selected_mod.name {
                            duplicate_name = true;
                        }
                    }
                    if duplicate_name {
                        ui.memory_mut(|mem|{
                            mem.toggle_popup(error_id);
                        });
                        self.log.add_to_log(LogType::Error, "A mod with that name already exists!".to_owned());
                    }
                    else {
                        self.mod_edit.path = Path::join(&self.mods_path, &self.mod_edit.name);
                        match fs::rename(self.mod_datas[selected_index].path.clone(), self.mod_edit.path.clone())
                        {
                            Ok(_) => {
                                let final_mod: ModData = self.mod_edit.clone();
                                match self.mod_edit.write_data() {
                                    Ok(()) => {
                                        if final_mod.name != self.mod_datas[selected_index].name {
                                            let mut config = CONFIG.lock().unwrap();
                                            remove_mod_config(self.mod_datas[selected_index].name.clone());
                                            self.write_config(&mut config);
                                            self.mod_datas[selected_index] = final_mod;
                                            self.log.add_to_log(LogType::Info, "Mod updated!".to_owned());
                                            self.set_mod_order_config(&mut config);
                                            window.edit_open = false;
                                        }
                                    },
                                    Err(e) => 
                                    {
                                        ui.memory_mut(|mem|{
                                            mem.toggle_popup(error_id);
                                        });        
                                        self.log.add_to_log(LogType::Error, format!("Could not edit mod! {}", e))
                                    }
                                }
                            }
                            Err(e) => self.log.add_to_log(LogType::Error, format!("Could not rename directory for edited mod! {}", e)),
                        }
                    }
                }
            }
        });
    
        window.edit_open &= edit_open;
    
        let mut remove_open: bool = window.remove_open;
        
        egui::Window::new("Remove Mod")
        .open(&mut remove_open)
        .show(ctx, |ui| {
            ui.label(RichText::new("WARNING").color(Color32::RED).size(32.));
            ui.label(RichText::new(format!("Are you sure you wish to remove {}?", self.selected_mod.name)).size(16.));
            ui.label(RichText::new("This action cannot be undone!").color(Color32::RED).size(16.));
    
            ui.horizontal(|ui|{
                if ui.button("Delete").clicked() {
                    match fs::remove_dir_all(self.mod_datas[selected_index].path.clone())
                    {
                        Ok(_) => {
                            remove_mod_config(self.mod_datas[selected_index].name.clone());
                            let mut config = CONFIG.lock().unwrap();
                            self.set_mod_order_config(&mut config);
                            self.write_config(&mut config);
                            self.mod_datas.remove(selected_index);
                            window.remove_open = false;
                        }
                        Err(e) => self.log.add_to_log(LogType::Error, format!("Could not remove mod! {}", e)),
                    }
                }
                if ui.button("Cancel").clicked() {
                    window.remove_open = false;
                }
            })
        });
        
        window.remove_open &= remove_open;
    
        egui::Window::new("About")
        .open(&mut window.about_open)
        .show(ctx, |ui| {
            ui.label(RichText::new("GUILTY GEAR Xrd Mod Manager").size(30.));
            ui.label(format!("Version {}", cargo_crate_version!()))
        });

        self.update_mods();
    }

    fn on_close_event(&mut self) -> bool {
        let mut config = CONFIG.lock().unwrap();
        self.set_mod_order_config(&mut config);
        self.write_config(&mut config);
        true
    }        
}

fn help_menu(ui: &mut Ui)
{
    if ui.button("About").clicked() {
        WINDOW.lock().unwrap().about_open = true;
        ui.close_menu();
    }
}