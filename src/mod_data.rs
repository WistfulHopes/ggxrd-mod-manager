use std::{path::{PathBuf, Path}, fs};
use ini::Ini;
use std::hash::{Hash, Hasher};

#[derive(Clone, Default)]
pub struct ModData {
    pub name: String,
    pub author: String,
    pub version: String,
    pub category: String,
    pub description: String,
    pub page: String,
    pub path: PathBuf,
    pub enabled: bool,
    pub order: usize,
    pub scripts: Vec<String>,
}

impl Hash for ModData {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state)
    }
}

impl ModData {
    pub fn new() -> ModData {
        ModData {
            name: "New Mod".to_owned(), 
            author: "".to_owned(), 
            version: "".to_owned(), 
            category: "".to_owned(), 
            description: "".to_owned(), 
            page: "".to_owned(), 
            path: PathBuf::new(),
            enabled: true, 
            order: 0,
            scripts: Vec::new(),
        }
    }

    pub fn write_data(&self) -> std::io::Result<()> 
    {
        fs::create_dir_all(&self.path)?;
        let mut conf = Ini::new();
        conf.with_section(Some("Description"))
            .set("Name", &self.name)
            .set("Author", &self.author)
            .set("Version", &self.version)
            .set("Category", &self.category)
            .set("Description", &self.description)
            .set("Page", &self.page);

        for script in &self.scripts {
            conf.with_section(Some("Scripts")).set("ScriptPackage", script);
        }

        conf.write_to_file(Path::join(&self.path, "mod.ini"))?;

        Ok(())
    }
}