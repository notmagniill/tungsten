use anyhow::{Result, Context};
use serde::{Serialize, Deserialize};
use std::collections::HashMap;

const LOCKFILE_PATH: &str = "tungsten.lock.toml";
const LOCKFILE_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Default)]
pub struct Lockfile {
    pub version: u32,
    pub inputs: HashMap<String, HashMap<String, LockfileEntry>>,
}

#[derive(Serialize, Deserialize)]
pub struct LockfileEntry {
    pub asset_id: u64,
}

impl Lockfile {
    pub fn load() -> Result<Lockfile> {
        let path = std::path::Path::new(LOCKFILE_PATH);
        
        if !path.exists() {
            return Ok(Lockfile {
                version: LOCKFILE_VERSION,
                inputs: HashMap::new(),
            });
        }
    
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Could not read \"{}\"", LOCKFILE_PATH))?;
    
        let lockfile: Lockfile = toml::from_str(&content)
            .with_context(|| "Failed to parse lockfile — it may be corrupted, try deleting it and re-running")?;
    
        Ok(lockfile)
    }
    
    pub fn save(&self) -> Result<()> {
        let content = toml::to_string(self)
            .with_context(|| "Failed to serialize lockfile")?;
    
        std::fs::write(LOCKFILE_PATH, content)
            .with_context(|| format!("Could not write \"{}\"", LOCKFILE_PATH))?;
    
        Ok(())
    }
    
    pub fn get(&self, input_name: &str, hash: &str) -> Option<u64> {
        self.inputs
            .get(input_name)?
            .get(hash)
            .map(|entry| entry.asset_id)
    }
    
    pub fn set(&mut self, input_name: &str, hash: String, asset_id: u64) {
        self.inputs
            .entry(input_name.to_string())
            .or_insert_with(HashMap::new)
            .insert(hash, LockfileEntry { asset_id });
    }
}

pub fn hash_image(data: &[u8]) -> String {
    use sha2::{Sha256, Digest};
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect()
}