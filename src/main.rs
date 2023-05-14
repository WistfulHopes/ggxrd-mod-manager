#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{path::{PathBuf, Path}, fs, ffi::OsStr, io::Cursor, process::{Command, exit}};

use bevy::{prelude::*, window::PresentMode};
use bevy_egui::{egui::{self, text::LayoutJob, TextFormat, FontId, FontFamily, Color32, Ui, RichText}, EguiContexts, EguiPlugin};
use egui_dnd::{DragDropUi, utils::shift_vec};
use ini::{Ini, EscapePolicy};
use log::{Log, LogType};
use mod_data::ModData;
use self_update::cargo_crate_version;
use steamlocate::SteamDir;

mod mod_data;
mod log;
mod helpers;

fn main() {
    App::new()
        .init_resource::<ManagerState>()
        .init_resource::<WindowState>()
        .init_resource::<ConfigState>()
        .add_plugins(DefaultPlugins.set(WindowPlugin{
            primary_window: Some(Window {
                title: "GUILTY GEAR Xrd Mod Manager".into(),
                resolution: (1280., 720.).into(),
                present_mode: PresentMode::AutoVsync,
                // Tells wasm to resize the window according to the available canvas
                fit_canvas_to_parent: true,
                // Tells wasm not to override default event handling, like F5, Ctrl+R etc.
                prevent_default_event_handling: false,
                ..default()
            }),
            ..default()
        }))
        .add_plugin(EguiPlugin)
        .add_startup_system(init_log)

        .add_startup_system(init_update)
        .add_startup_system(init_config)
        .add_startup_system(init_steam)
        .add_startup_system(init_mods)
        .add_startup_system(configure_visuals_system)
        .add_startup_system(configure_ui_state_system)
        .add_system(ui_system)
        .run();
}

fn init_update(mut ui_state: ResMut<ManagerState>) {
    match helpers::update() {
        Ok(status) => {
            match status {
                self_update::Status::UpToDate(_) => ui_state.log.add_to_log(LogType::Info, "You are on the latest version!".to_owned()),
                self_update::Status::Updated(_) => 
                {
                    ui_state.log.add_to_log(LogType::Info, "Update successful! Restarting...".to_owned());
                    Command::new("ggxrd-mod-manager-x86_64-pc-windows-msvc.exe").spawn().unwrap();
                    exit(0)
                }
            }
        }
        Err(e) => ui_state.log.add_to_log(LogType::Error, format!("Update failed! {}", e)),
    }
}

#[derive(Default, Resource)]
struct ConfigState {
    config: Ini,
}

#[derive(Default, Resource)]
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

impl ManagerState {
    fn mods_layout(&mut self, ui: &mut Ui, config_state: &mut ResMut<ConfigState>, window_state: &mut ResMut<WindowState>) -> (bool, bool)
    {
        let mut config_needs_update = false;
        let mut edit_flag = false;
        let response = self.dnd.ui::<ModData>(ui, self.mod_datas.iter_mut(), |mod_data, ui, handle| {
            ui.horizontal(|ui| {
                if ui.checkbox(&mut mod_data.enabled, "").changed() {
                    update_mod_config(config_state, mod_data.name.clone(), mod_data);
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
                    ui.set_min_width(150.);
                    if ui.button("Open containing folder").clicked() {
                        open::that(mod_data.path.clone()).unwrap_or_default();
                    }
                    if ui.button("Edit mod").clicked() {
                        window_state.edit_open = true;
                        edit_flag = true;
                    }
                    if ui.button("Remove mod").clicked() {
                        window_state.remove_open = true;
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
        }
        (config_needs_update, edit_flag)
    }
}

#[derive(Default, Resource)]
struct WindowState {
    about_open: bool,
    create_open: bool,
    edit_open: bool,
    remove_open: bool,
}

fn init_steam(mut ui_state: ResMut<ManagerState>)
{
    let steamdir: Option<SteamDir> = SteamDir::locate();
    match steamdir {
        Some(mut dir) => {
            match dir.app(&520440)
            {
                Some(app) => {
                    ui_state.game_path = app.path.clone();
                    ui_state.log.add_to_log(LogType::Info, format!("Guilty Gear Xrd Rev 2 located at {}.", app.path.display()))
                },
                None => ui_state.log.add_to_log(LogType::Error, "Could not locate Guilty Gear Xrd Rev 2! Make sure you have it installed.".to_owned())
            }
        },
        None => ui_state.log.add_to_log(LogType::Error, "Could not locate Steam!".to_owned())
    }
}

fn init_config(mut ui_state: ResMut<ManagerState>, mut config_state: ResMut<ConfigState>)
{
    let ini_path = Path::new("config.ini");
    if ini_path.exists() {
        let config = Ini::load_from_file_noescape(ini_path);
        match config {
            Ok(ini) => config_state.config = ini,
            Err(_) => create_config(&mut ui_state, &mut config_state),
        }
    }
    else 
    {
        create_config(&mut ui_state, &mut config_state)
    } 
}

fn create_config(ui_state: &mut ResMut<ManagerState>, config_state: &mut ResMut<ConfigState>)
{
    let mut ini = Ini::new();
    ini.with_section(Some("General"))
        .set("ConsoleVisible", "True");
    write_config(ui_state, config_state)
}

fn write_config(ui_state: &mut ResMut<ManagerState>, config_state: &mut ResMut<ConfigState>)
{
    let ini_path = Path::new("config.ini");
    match config_state.config.write_to_file(ini_path)
    {
        Ok(_) => (),
        Err(e) => ui_state.log.add_to_log(LogType::Error, format!("Could not create config ini! {}", e))
    }
}

fn set_mod_order_config(ui_state: &mut ResMut<ManagerState>, config_state: &mut ResMut<ConfigState>)
{
    config_state.config.delete(Some("Mods"));
    for mod_data in &ui_state.mod_datas {
        let enabled = match mod_data.enabled {
            true => "True",
            false => "False",
        };
        config_state.config.with_section(Some("Mods"))
            .set(mod_data.name.clone(), enabled);
    }
    write_config(ui_state, config_state)
}

fn init_mod_config(config_state: &mut ResMut<ConfigState>, mod_name: String, data: &mut ModData)
{
    let section = config_state.config.section(Some("Mods"));
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
                    config_state.config.with_section(Some("Mods")).set(&mod_name, "True");
                }
            }        
        }
        None => {
            config_state.config.with_section(Some("Mods")).set(&mod_name, "True");
        }
    }
}

fn update_mod_config(config_state: &mut ResMut<ConfigState>, mod_name: String, data: &mut ModData)
{
    match data.enabled {
        true => {
            config_state.config.with_section(Some("Mods")).set(&mod_name, "True");
        }
        false => {
            config_state.config.with_section(Some("Mods")).set(&mod_name, "False");
        }
    }
}

fn remove_mod_config(config_state: &mut ResMut<ConfigState>, mod_name: String)
{
    config_state.config.delete(Some(mod_name));
}

fn init_mods(mut ui_state: ResMut<ManagerState>, mut config_state: ResMut<ConfigState>)
{
    ui_state.mods_path = Path::join(&std::env::current_dir().unwrap(), "Mods");
    let mod_section = config_state.config.section(Some("Mods"));
    match mod_section {
        Some(mod_section) => {
            for mod_entry in mod_section.iter() {
                let path = Path::join(&ui_state.mods_path, mod_entry.0).join("mod.ini");
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
                                            ui_state.log.add_to_log(LogType::Warn, format!("The mod ini at path {} doesn't have a name in the desciption section! Ignoring mod.", path.display()));
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

                                    mod_data.path = Path::join(&ui_state.mods_path, &mod_name.unwrap());
                                    mod_data.enabled = match mod_entry.1 {
                                        "True" => true,
                                        "False" => false,
                                        _ => true,
                                    };
                                    mod_data.order = ui_state.mod_datas.len();
                                    ui_state.mod_datas.push(mod_data);
                                },
                                None => {
                                    ui_state.log.add_to_log(LogType::Error, format!("The mod ini at path {} doesn't have a description section! Ignoring mod.", path.display()));
                                    continue
                                }
                            }
                        },
                        Err(_) => {
                            ui_state.log.add_to_log(LogType::Error, format!("Ini at path {} does not exist! Ignoring mod.", path.display()));
                            continue
                        }
                    }
                }
                else {
                    ui_state.log.add_to_log(LogType::Error, format!("Path {} does not exist! Ignoring mod.", path.display()));
                }
            }
        }
        None => ui_state.log.add_to_log(LogType::Warn, "No mods found in the config ini! You probably need to install a mod.".to_owned()),
    }
    for mod_data in &mut ui_state.mod_datas {
        init_mod_config(&mut config_state, mod_data.name.clone(), mod_data);
    }
    set_mod_order_config(&mut ui_state, &mut config_state);
    write_config(&mut ui_state, &mut config_state);
}


fn init_mod(ui_state: &mut ResMut<ManagerState>, config_state: &mut ResMut<ConfigState>, name: String)
{
    let path = Path::join(&ui_state.mods_path, &name).join("mod.ini");
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
                                ui_state.log.add_to_log(LogType::Warn, format!("The mod ini at path {} doesn't have a name in the desciption section! Ignoring mod.", path.display()));
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
                        mod_data.path = path.to_path_buf();
                        init_mod_config(config_state, mod_name.unwrap().to_owned(), &mut mod_data);
                        write_config(ui_state, config_state);
                        ui_state.mod_datas.push(mod_data);
                    },
                    None => {
                        mod_data.name = name.clone();
                        mod_data.path = path.to_path_buf();
                        mod_data.write_data().unwrap_or_default();
                        init_mod_config(config_state, name, &mut mod_data);
                        write_config(ui_state, config_state);
                        ui_state.mod_datas.push(mod_data);
                        ui_state.log.add_to_log(LogType::Warn, format!("The mod ini at path {} doesn't have a description section! Created one automatically.", &path.display()));
                    }
                }
            },
            Err(_) => {
                mod_data.name = name.clone();
                mod_data.path = path.to_path_buf();
                mod_data.write_data().unwrap_or_default();
                init_mod_config(config_state, name, &mut mod_data);
                write_config(ui_state, config_state);
                ui_state.mod_datas.push(mod_data);
                ui_state.log.add_to_log(LogType::Warn, format!("No mod ini at path {}! Created one automatically.", &path.display()));
            }
        }
    }
    else {
        ui_state.log.add_to_log(LogType::Warn, format!("Path {} does not exist! Ignoring mod.", &path.display()));
    }
}

fn configure_visuals_system(mut contexts: EguiContexts) {
    contexts.ctx_mut().set_visuals(egui::Visuals {
        window_rounding: 0.0.into(),
        ..Default::default()
    });
}

fn configure_ui_state_system(mut ui_state: ResMut<ManagerState>) {
    ui_state.console_visible = true;
}

fn init_log(mut ui_state: ResMut<ManagerState>) {
    ui_state.log.add_to_log(LogType::Info, "Launched GUILTY GEAR Xrd Mod Manager".to_owned());
}

fn ui_system(mut ui_state: ResMut<ManagerState>, 
    mut window_state: ResMut<WindowState>, 
    mut config_state: ResMut<ConfigState>, 
    mut contexts: EguiContexts) 
    {
    egui::TopBottomPanel::top("header_panel").show(contexts.ctx_mut(), |ui: &mut Ui| {
        ui.horizontal(|ui| {
            ui.menu_button("File", |ui| {
                file_menu(&mut ui_state, &mut config_state, &mut window_state, ui)
            });
            ui.menu_button("Settings", |ui| {
                settings_menu(&mut ui_state, ui)
            });
            ui.menu_button("Help", |ui| {
                help_menu(&mut window_state, ui)
            });
            let mut visuals = ui.ctx().style().visuals.clone();
            visuals.light_dark_radio_buttons(ui);
            ui.ctx().set_visuals(visuals);
        });
    });
    
    if ui_state.console_visible
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
        .show(contexts.ctx_mut(), |ui: &mut Ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                let mut log: &str = &ui_state.log.log_text;
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

    egui::SidePanel::left("options_panel").show(contexts.ctx_mut(), |ui: &mut Ui| {
        ui.vertical(|ui| {
            // TODO implement browsing functionality, and swapping between it and managing
            /*if ui.small_button("🌐 Browse Mods").clicked() {

            }
            if ui.small_button("📁 Manage Mods").clicked() {

            }*/
            if ui.small_button("▶️ Launch Game").clicked() {
                setup_mods_and_play(&mut ui_state);
            }
        });
    });

    egui::SidePanel::right("details_panel")
        .max_width(f32::INFINITY)
        .min_width(280.)
        .show(contexts.ctx_mut(), |ui: &mut Ui| {
            ui.vertical(|ui| {
                ui.label(format!("Author: {}", ui_state.selected_mod.author));
                ui.label(format!("Category: {}", ui_state.selected_mod.category));
                ui.label(format!("Description: {}", &ui_state.selected_mod.description));
                ui.label(format!("Version: {}", ui_state.selected_mod.version));
            });
    });

    let mut config_needs_update = false;

    let mut edit_flag = false;

    egui::CentralPanel::default().show(contexts.ctx_mut(), |ui| {
        let mods_return_value = ui_state.mods_layout(ui, &mut config_state, &mut window_state);
        config_needs_update = mods_return_value.0;
        edit_flag = mods_return_value.1;
    });

    let mut selected_index: usize = usize::MAX;
    for (index, data) in ui_state.mod_datas.iter().enumerate() {
        if data.name == ui_state.selected_mod.name {
            selected_index = index;
            break;
        }
    }

    if edit_flag {
        ui_state.mod_edit = ui_state.mod_datas[selected_index].clone();
    }

    if config_needs_update {
        set_mod_order_config(&mut ui_state, &mut config_state);
        write_config(&mut ui_state, &mut config_state)
    }

    let mut create_open: bool = window_state.create_open;

    egui::Window::new("Create Mod")
    .open(&mut create_open)
    .show(contexts.ctx_mut(), |ui| {
        ui.label(RichText::new("Fill out details about your mod.").size(18.));

        ui.label("Name");
        ui.text_edit_singleline(&mut ui_state.mod_edit.name);
        ui.end_row();

        ui.label("Author");
        ui.text_edit_singleline(&mut ui_state.mod_edit.author);
        ui.end_row();

        ui.label("Category");
        ui.text_edit_singleline(&mut ui_state.mod_edit.category);
        ui.end_row();

        ui.label("Version");
        ui.text_edit_singleline(&mut ui_state.mod_edit.version);
        ui.end_row();

        ui.label("Description");
        ui.text_edit_singleline(&mut ui_state.mod_edit.description);
        ui.end_row();

        ui.label("UnrealScript Packages");
        for script in &mut ui_state.mod_edit.scripts {
            ui.text_edit_singleline(script);
        }
        if ui.button("➕").clicked() {
            ui_state.mod_edit.scripts.push("".to_owned());
        }
        if ui.button("➖").clicked() {
            ui_state.mod_edit.scripts.pop();
        }
        ui.end_row();

        let ok_response = ui.button("OK");
        let error_id = ui.make_persistent_id("error");

        egui::popup::popup_below_widget(ui, error_id, &ok_response, |ui| {
            ui.set_min_width(150.);
            ui.label("Creation failed! Check log for more details.");
        });

        if ok_response.clicked() {
            if ui_state.mod_edit.name.is_empty()
            {
                ui.memory_mut(|mem|{
                    mem.toggle_popup(error_id);
                });
                ui_state.log.add_to_log(LogType::Error, "You must give your mod a name!".to_owned());
            }
            else {
                let mut duplicate_name = false;
                for data in &ui_state.mod_datas {
                    if data.name == ui_state.mod_edit.name {
                        duplicate_name = true;
                    }
                }
                if duplicate_name {
                    ui.memory_mut(|mem|{
                        mem.toggle_popup(error_id);
                    });
                    ui_state.log.add_to_log(LogType::Error, "A mod with that name already exists!".to_owned());
                }
                else {
                    ui_state.mod_edit.order = ui_state.mod_datas.len();
                    ui_state.mod_edit.path = Path::join(&ui_state.mods_path, &ui_state.mod_edit.name);
                    let final_mod: ModData = ui_state.mod_edit.clone();
                    match ui_state.mod_edit.write_data() {
                        Ok(()) => {
                            ui_state.log.add_to_log(LogType::Info, format!("Created mod {}!", &final_mod.name));
                            ui_state.mod_datas.push(final_mod.clone());
                            set_mod_order_config(&mut ui_state, &mut config_state);
                            window_state.create_open = false;
                            open::that(final_mod.path.clone()).unwrap_or_default();
                        },
                        Err(e) => 
                        {
                            ui.memory_mut(|mem|{
                                mem.toggle_popup(error_id);
                            });        
                            ui_state.log.add_to_log(LogType::Error, format!("Could not create mod! {}", e))
                        }
                    }
                }
            }
        }
    });

    window_state.create_open &= create_open;

    let mut edit_open: bool = window_state.edit_open;

    egui::Window::new("Edit Mod")
    .open(&mut edit_open)
    .show(contexts.ctx_mut(), |ui| {
        ui.label(RichText::new("Fill out details about your mod.").size(18.));

        ui.label("Name");
        ui.text_edit_singleline(&mut ui_state.mod_edit.name);
        ui.end_row();

        ui.label("Author");
        ui.text_edit_singleline(&mut ui_state.mod_edit.author);
        ui.end_row();

        ui.label("Category");
        ui.text_edit_singleline(&mut ui_state.mod_edit.category);
        ui.end_row();

        ui.label("Version");
        ui.text_edit_singleline(&mut ui_state.mod_edit.version);
        ui.end_row();

        ui.label("Description");
        ui.text_edit_singleline(&mut ui_state.mod_edit.description);
        ui.end_row();

        ui.label("UnrealScript Packages");
        for script in &mut ui_state.mod_edit.scripts {
            ui.text_edit_singleline(script);
        }
        if ui.button("➕").clicked() {
            ui_state.mod_edit.scripts.push("".to_owned());
        }
        if ui.button("➖").clicked() {
            ui_state.mod_edit.scripts.pop();
        }
        ui.end_row();

        let ok_response = ui.button("OK");
        let error_id = ui.make_persistent_id("error_edit");

        egui::popup::popup_below_widget(ui, error_id, &ok_response, |ui| {
            ui.set_min_width(150.);
            ui.label("Creation failed! Check log for more details.");
        });

        if ok_response.clicked() {
            if ui_state.mod_edit.name.is_empty()
            {
                ui.memory_mut(|mem|{
                    mem.toggle_popup(error_id);
                });
                ui_state.log.add_to_log(LogType::Error, "You must give your mod a name!".to_owned());
            }
            else {
                let mut duplicate_name = false;
                for data in &ui_state.mod_datas {
                    if data.name == ui_state.mod_edit.name && data.name != ui_state.selected_mod.name {
                        duplicate_name = true;
                    }
                }
                if duplicate_name {
                    ui.memory_mut(|mem|{
                        mem.toggle_popup(error_id);
                    });
                    ui_state.log.add_to_log(LogType::Error, "A mod with that name already exists!".to_owned());
                }
                else {
                    ui_state.mod_edit.path = Path::join(&ui_state.mods_path, &ui_state.mod_edit.name);
                    match fs::rename(ui_state.mod_datas[selected_index].path.clone(), ui_state.mod_edit.path.clone())
                    {
                        Ok(_) => {
                            let final_mod: ModData = ui_state.mod_edit.clone();
                            match ui_state.mod_edit.write_data() {
                                Ok(()) => {
                                    if final_mod.name != ui_state.mod_datas[selected_index].name {
                                        remove_mod_config(&mut config_state, ui_state.mod_datas[selected_index].name.clone());
                                        write_config(&mut ui_state, &mut config_state);
                                        ui_state.mod_datas[selected_index] = final_mod;
                                        ui_state.log.add_to_log(LogType::Info, "Mod updated!".to_owned());
                                        set_mod_order_config(&mut ui_state, &mut config_state);
                                        window_state.edit_open = false;            
                                    }
                                },
                                Err(e) => 
                                {
                                    ui.memory_mut(|mem|{
                                        mem.toggle_popup(error_id);
                                    });        
                                    ui_state.log.add_to_log(LogType::Error, format!("Could not edit mod! {}", e))
                                }
                            }        
                        }
                        Err(e) => ui_state.log.add_to_log(LogType::Error, format!("Could not rename directory for edited mod! {}", e)),
                    }
                }
            }
        }
    });

    window_state.edit_open &= edit_open;

    let mut remove_open: bool = window_state.remove_open;
    
    egui::Window::new("Remove Mod")
    .open(&mut remove_open)
    .show(contexts.ctx_mut(), |ui| {
        ui.label(RichText::new("WARNING").color(Color32::RED).size(32.));
        ui.label(RichText::new(format!("Are you sure you wish to remove {}?", ui_state.selected_mod.name)).size(16.));
        ui.label(RichText::new("This action cannot be undone!").color(Color32::RED).size(16.));

        ui.horizontal(|ui|{
            if ui.button("Delete").clicked() {
                match fs::remove_dir_all(ui_state.mod_datas[selected_index].path.clone())
                {
                    Ok(_) => {
                        remove_mod_config(&mut config_state, ui_state.mod_datas[selected_index].name.clone());
                        set_mod_order_config(&mut ui_state, &mut config_state);
                        write_config(&mut ui_state, &mut config_state);
                        ui_state.mod_datas.remove(selected_index);
                        window_state.remove_open = false;
                    }
                    Err(e) => ui_state.log.add_to_log(LogType::Error, format!("Could not remove mod! {}", e)),
                }
            }
            if ui.button("Cancel").clicked() {
                window_state.remove_open = false;
            }
        })
    });
    
    window_state.remove_open &= remove_open;

    egui::Window::new("About")
    .open(&mut window_state.about_open)
    .show(contexts.ctx_mut(), |ui| {
        ui.label(RichText::new("GUILTY GEAR Xrd Mod Manager").size(30.));
        ui.label(format!("Version {}", cargo_crate_version!()))
    });
}

fn setup_mods_and_play(ui_state: &mut ManagerState)
{
    let ini_path = Path::join(&ui_state.game_path, "REDGame").join("Config").join("DefaultEngine.ini");
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
                        Err(e) => ui_state.log.add_to_log(LogType::Error, format!("Could not write to DefaultEngine.ini! {}", e)),
                    }    
                }
                None => ui_state.log.add_to_log(LogType::Error, "Could not find Engine.ScriptPackages in DefaultEngine.ini! Your game installation may be broken.".to_owned()),
            }
    }
        Err(e) => ui_state.log.add_to_log(LogType::Error, format!("Could not read DefaultEngine.ini! {}", e)),
    }
    fs::remove_dir_all(Path::join(&ui_state.game_path, "REDGame").join("CookedPCConsole").join("Mods")).unwrap_or_default();
    for mod_data in ui_state.mod_datas.iter().rev() {
        if mod_data.enabled {
            let mut folder_string = "a".to_owned();
            let game_mods_path = Path::join(&ui_state.game_path, "REDGame").join("CookedPCConsole").join("Mods");
            while Path::join(&game_mods_path, &folder_string).exists() {
                let tmp_string = helpers::add1_str(&folder_string);
                if folder_string != tmp_string {
                    folder_string = tmp_string;
                }
                else {
                    ui_state.log.add_to_log(LogType::Error, format!("Could not copy mod {}! Too many mods installed.", &mod_data.name));
                    break;
                }
            }
            match helpers::copy_recursively(&mod_data.path, Path::join(&game_mods_path, &folder_string))
            {
                Ok(_) => (),
                Err(e) => {
                    ui_state.log.add_to_log(LogType::Error, format!("Could not copy mod {}! {}", &mod_data.name, e));
                    continue;
                }
            }
            let ini_path: PathBuf = Path::join(&ui_state.game_path, "REDGame").join("Config").join("DefaultEngine.ini");
            let ini: Result<Ini, ini::Error> = Ini::load_from_file_noescape(&ini_path);
            match ini {
                Ok(mut ini) => {
                    for script in &mod_data.scripts {
                        match ini.section_mut(Some("Engine.ScriptPackages"))
                        {
                            Some(section) => {
                                if section.get_all("+NativePackages").find(|x| x == script).is_none() {
                                    section.append("+NativePackages", script);
                                    ui_state.log.add_to_log(LogType::Info, format!("Added script package {}!", script))
                                }
                            }
                            None => ui_state.log.add_to_log(LogType::Error, "Could not read find Engine.ScriptPackages in DefaultEngine.ini! Your game installation may be broken.".to_owned()),
                        }
                    }
                    match ini.write_to_file_policy(&ini_path, EscapePolicy::Nothing) {
                        Ok(_) => (),
                        Err(e) => ui_state.log.add_to_log(LogType::Error, format!("Could not write to DefaultEngine.ini! {}", e)),
                    }
                }
                Err(e) => ui_state.log.add_to_log(LogType::Error, format!("Could not read DefaultEngine.ini! {}", e)),
            }    
        }
    }
    ui_state.log.add_to_log(LogType::Info, "Mods copied to game directory!".to_string());
    match open::that("steam://run/520440")
    {
        Ok(_) => ui_state.log.add_to_log(LogType::Info, "Launching Guilty Gear Xrd Rev 2...".to_string()),
        Err(e) => ui_state.log.add_to_log(LogType::Error, format!("Could not launch Guilty Gear Xrd Rev 2! {}", e)),
    }
}

fn file_menu(ui_state: &mut ResMut<ManagerState>, config_state: &mut ResMut<ConfigState>, window_state: &mut ResMut<WindowState>, ui: &mut Ui)
{
    if ui.button("Install Mod").clicked() {
        if let Some(path) = rfd::FileDialog::new()
        .add_filter("All supported archives", &["zip", "rar", "7z"])
        .add_filter("ZIP archive", &["zip"])
        .add_filter("7Z archive", &["7z"])
        .add_filter("RAR archive", &["rar"])
        .pick_file() {
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
                    ui_state.log.add_to_log(LogType::Error, "File has no name!".to_owned());
                    return
                }
            };
            match file_type {
                0 => {
                    match std::fs::read(&path) {
                        Ok(bytes) => {
                            match zip_extract::extract(Cursor::new(bytes), 
                                &Path::join(&ui_state.mods_path, file_stem), true)
                            {
                                Ok(_) => {
                                    init_mod(ui_state, config_state, file_stem.to_str().unwrap().to_owned())
                                },
                                Err(e) => ui_state.log.add_to_log(LogType::Error, format!("Could not extract archive! {}", e))
                            }
                        }
                        Err(e) => {
                            ui_state.log.add_to_log(LogType::Error, format!("Could not read archive! {}", e))
                        }
                    }
                }
                1 => {
                    match sevenz_rust::decompress_file(&path, Path::join(&ui_state.mods_path, file_stem))
                    {
                        Ok(_) => {
                            init_mod(ui_state, config_state, file_stem.to_str().unwrap().to_owned())
                        },
                        Err(e) => ui_state.log.add_to_log(LogType::Error, format!("Could not extract archive! {}", e))
                    }        
                }
                2 => {
                    match unrar::Archive::new(&path) {
                        Ok(archive) => 
                        {
                            match archive.extract_to(Path::join(&ui_state.mods_path, file_stem))
                            {
                                Ok(mut archive) => {
                                    match archive.process() {
                                        Ok(_) => init_mod(ui_state, config_state, file_stem.to_str().unwrap().to_owned()),
                                        Err(e) => ui_state.log.add_to_log(LogType::Error, format!("Could not extract archive! {}", e))
                                    }
                                },
                                Err(e) => ui_state.log.add_to_log(LogType::Error, format!("Could not extract archive! {}", e))
                            }        
                        }
                        Err(e) => {
                            ui_state.log.add_to_log(LogType::Error, format!("Could not read archive! {}", e))
                        }
                    }
                }
                _ => {
                    ui_state.log.add_to_log(LogType::Error, "Invalid file extension!".to_string())
                }
            }
        };
        ui.close_menu();
    }
    if ui.button("Create Mod").clicked() {
        window_state.create_open = true;
        ui.close_menu();
    }
}

fn settings_menu(ui_state: &mut ResMut<ManagerState>, ui: &mut Ui)
{
    if ui.checkbox(&mut ui_state.console_visible, "Show Console").changed() {
        ui.close_menu();
    }
}

fn help_menu(window_state: &mut ResMut<WindowState>, ui: &mut Ui)
{
    if ui.button("About").clicked() {
        window_state.about_open = true;
        ui.close_menu();
    }
}