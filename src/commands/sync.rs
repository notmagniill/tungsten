use anyhow::{Result, Context, bail};
use glob::glob;
use std::path::PathBuf;
use crate::utils::config::Config;
use crate::utils::logger::progress;
use crate::api::upload::RobloxClient;
use crate::api::roblox::{Creator, UserCreator, GroupCreator};
use crate::core::{pack, codegen};
use crate::core::codegen::CodegenEntry;
use crate::core::lockfile::{Lockfile, hash_image};
use crate::log;

pub async fn run(config: Config, api_key: Option<String>, target: &str) -> Result<()> {
    let mut errors: u32 = 0;

    let mut lockfile = Lockfile::load()
        .context("Failed to load lockfile")?;

    let client = if target == "roblox" {
        let key = api_key.as_deref()
            .ok_or_else(|| anyhow::anyhow!(
                "Missing --api-key flag\n  Hint: Generate an API key at https://create.roblox.com/credentials with \"Assets: Read & Write\" permissions"
            ))?;
        Some(RobloxClient::new(key.to_string()))
    } else {
        None
    };

    let creator = match config.creator.creator_type.as_str() {
        "user" => Creator::User(UserCreator {
            user_id: config.creator.id.to_string(),
        }),
        "group" => Creator::Group(GroupCreator {
            group_id: config.creator.id.to_string(),
        }),
        other => bail!(
            "Invalid creator type \"{}\"\n  Hint: Must be \"user\" or \"group\"",
            other
        ),
    };

    let codegen_style = config.codegen
        .as_ref()
        .and_then(|c| c.style.as_deref())
        .unwrap_or("flat")
        .to_string();

    let strip_extension = config.codegen
        .as_ref()
        .and_then(|c| c.strip_extension)
        .unwrap_or(false);

    for (input_name, input) in &config.inputs {
        log!(section, "Processing \"{}\"", input_name);

        // Step 1: Resolve glob
        let paths: Vec<PathBuf> = glob(&input.path)
            .with_context(|| format!(
                "Invalid glob pattern \"{}\"\n  Hint: Example: path = \"assets/**/*.png\"",
                input.path
            ))?
            .filter_map(|entry| match entry {
                Ok(path) if path.extension().map(|e| e == "png").unwrap_or(false) => Some(path),
                Ok(_) => None,
                Err(e) => {
                    log!(warn, "Skipping unreadable path: {}", e);
                    None
                }
            })
            .collect();

        if paths.is_empty() {
            log!(warn, "No PNG files matched \"{}\" — skipping", input.path);
            continue;
        }

        log!(info, "Found {} PNG files", paths.len());

        // Step 2: Load images
        let base_path = input.path
            .split('*')
            .next()
            .unwrap_or("")
            .trim_end_matches('/')
            .to_string();
         
        log!(info, "Loading images...");
        
        let images = match pack::load_images(paths, &base_path) {
            Ok(imgs) => imgs,
            Err(e) => {
                log!(warn, "Failed to load images for \"{}\": {}", input_name, e);
                errors += 1;
                continue;
            }
        };

        let should_pack = input.packable.unwrap_or(false);

        if should_pack {
            // Step 3: Pack
            log!(info, "Packing into spritesheets...");
            let spritesheets = match pack::pack(images) {
                Ok(s) => s,
                Err(e) => {
                    log!(warn, "Failed to pack images for \"{}\": {}", input_name, e);
                    errors += 1;
                    continue;
                }
            };

            log!(success, "Packed into {} spritesheet(s)", spritesheets.len());

            let mut codegen_entries: Vec<CodegenEntry> = Vec::new();

            for (sheet_index, sheet) in spritesheets.iter().enumerate() {
                let mut png_bytes: Vec<u8> = Vec::new();
                let encoder = image::codecs::png::PngEncoder::new(std::io::Cursor::new(&mut png_bytes));
                if let Err(e) = image::ImageEncoder::write_image(
                    encoder,
                    sheet.image.as_raw(),
                    sheet.image.width(),
                    sheet.image.height(),
                    image::ExtendedColorType::Rgba8,
                ) {
                    log!(warn, "Failed to encode spritesheet #{}: {}", sheet_index + 1, e);
                    errors += 1;
                    continue;
                }

                let hash = hash_image(&png_bytes);

                let asset_id = if let Some(ref client) = client {
                    if let Some(cached_id) = lockfile.get(input_name, &hash) {
                        log!(info, "Spritesheet #{} unchanged, skipping upload (rbxassetid://{})", sheet_index + 1, cached_id);
                        cached_id
                    } else {
                        log!(info, "Uploading spritesheet #{}...", sheet_index + 1);
                        match client.upload(
                            &format!("tungsten_{}_{}", input_name, sheet_index),
                            png_bytes.clone(),
                            creator.clone(),
                        ).await {
                            Ok(id) => {
                                lockfile.set(input_name, hash, id);
                                if let Err(e) = lockfile.save() {
                                    log!(warn, "Failed to save lockfile: {}", e);
                                    errors += 1;
                                }
                                log!(success, "Spritesheet #{} uploaded → rbxassetid://{}", sheet_index + 1, id);
                                id
                            }
                            Err(e) => {
                                log!(warn, "Failed to upload spritesheet #{}: {}", sheet_index + 1, e);
                                errors += 1;
                                continue;
                            }
                        }
                    }
                } else {
                    log!(info, "Dry run: skipping upload for spritesheet #{}", sheet_index + 1);
                    0
                };

                for img in &sheet.images {
                    codegen_entries.push(CodegenEntry {
                        name: img.name.clone(),
                        asset_id,
                        rect_offset: (img.x, img.y),
                        rect_size: (img.width, img.height),
                    });
                }
            }

            let table_name = std::path::Path::new(&input.output_path)
                .file_stem()
                .with_context(|| format!("Invalid output path \"{}\"", input.output_path))?
                .to_string_lossy()
                .to_string();

            log!(info, "Writing codegen to \"{}\"...", input.output_path);
            if let Err(e) = codegen::generate(
                codegen_entries,
                &table_name,
                &codegen_style,
                strip_extension,
                &input.output_path,
            ) {
                log!(warn, "Failed to write codegen for \"{}\": {}", input_name, e);
                errors += 1;
            } else {
                log!(success, "Codegen written to \"{}\"", input.output_path);
            }

        } else {
            // Not packable — upload individually
            log!(info, "Uploading images individually...");
            let mut codegen_entries: Vec<CodegenEntry> = Vec::new();
            let total = images.len();

            for (i, img) in images.into_iter().enumerate() {
                let mut png_bytes: Vec<u8> = Vec::new();
                let encoder = image::codecs::png::PngEncoder::new(std::io::Cursor::new(&mut png_bytes));
                if let Err(e) = image::ImageEncoder::write_image(
                    encoder,
                    img.image.as_raw(),
                    img.image.width(),
                    img.image.height(),
                    image::ExtendedColorType::Rgba8,
                ) {
                    log!(warn, "Failed to encode \"{}\": {}", img.name, e);
                    errors += 1;
                    continue;
                }

                let hash = hash_image(&png_bytes);

                let asset_id = if let Some(ref client) = client {
                    if let Some(cached_id) = lockfile.get(input_name, &hash) {
                        cached_id
                    } else {
                        match client.upload(&img.name, png_bytes, creator.clone()).await {
                            Ok(id) => {
                                lockfile.set(input_name, hash, id);
                                if let Err(e) = lockfile.save() {
                                    log!(warn, "Failed to save lockfile: {}", e);
                                    errors += 1;
                                }
                                id
                            }
                            Err(e) => {
                                log!(warn, "Failed to upload \"{}\": {}", img.name, e);
                                errors += 1;
                                continue;
                            }
                        }
                    }
                } else {
                    0
                };

                progress(i + 1, total, &img.name);

                codegen_entries.push(CodegenEntry {
                    name: img.name,
                    asset_id,
                    rect_offset: (0, 0),
                    rect_size: (img.image.width(), img.image.height()),
                });
            }

            let table_name = std::path::Path::new(&input.output_path)
                .file_stem()
                .with_context(|| format!("Invalid output path \"{}\"", input.output_path))?
                .to_string_lossy()
                .to_string();

            log!(info, "Writing codegen to \"{}\"...", input.output_path);
            if let Err(e) = codegen::generate(
                codegen_entries,
                &table_name,
                &codegen_style,
                strip_extension,
                &input.output_path,
            ) {
                log!(warn, "Failed to write codegen for \"{}\": {}", input_name, e);
                errors += 1;
            } else {
                log!(success, "Codegen written to \"{}\"", input.output_path);
            }
        }
    }

    log!(section, "Done");

    if errors > 0 {
        log!(warn, "Sync completed with {} error(s) — some assets may not have been uploaded", errors);
    } else {
        log!(success, "Tungsten sync complete!");
    }

    Ok(())
}
