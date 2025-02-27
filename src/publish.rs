//
// Copyright 2024 Formata, Inc. All rights reserved.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//    http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//

use std::{collections::HashSet, fs::{self, File}, io::{Read, Seek, Write}, path::Path, sync::Arc};
use anyhow::Context;
use bytes::Bytes;
use colored::Colorize;
use nanoid::nanoid;
use regex::Regex;
use stof::{SDoc, SField, SNodeRef, SVal};
use tokio::{sync::Mutex, task::JoinSet};
use walkdir::{DirEntry, WalkDir};
use zip::write::SimpleFileOptions;


/// Publish a stof package to registries.
pub(crate) async fn publish_package(dir: &str) {
    let pkg_path = format!("{}/pkg.stof", dir);
    if let Ok(pkg_doc) = SDoc::file(&pkg_path, "stof") {
        let mut pkg_path = String::default();
        let mut publish_registries = Vec::new();
        let mut excluded = HashSet::new();

        if let Some(name_field) = SField::field(&pkg_doc.graph, "root.name", '.', None) {
            let pkg_name = name_field.to_string();
            pkg_path = pkg_name.trim_start_matches("@").to_owned();

            if let Some(publish_array) = SField::field(&pkg_doc.graph, "root.publish", '.', None) {
                match &publish_array.value {
                    SVal::Array(vals) => {
                        for val in vals {
                            match val {
                                SVal::Object(nref) => {
                                    publish_registries.push(nref.clone());
                                },
                                _ => {}
                            }
                        }
                    },
                    _ => {}
                }
            }

            if let Some(exclude_array) = SField::field(&pkg_doc.graph, "root.exclude", '.', None) {
                match &exclude_array.value {
                    SVal::Array(vals) => {
                        for val in vals {
                            match val {
                                SVal::String(exclude) => {
                                    excluded.insert(exclude.clone());
                                },
                                _ => {}
                            }
                        }
                    },
                    SVal::Set(vals) => {
                        for val in vals {
                            match val {
                                SVal::String(exclude) => {
                                    excluded.insert(exclude.clone());
                                },
                                _ => {}
                            }
                        }
                    },
                    _ => {}
                }
            }
        }

        if publish_registries.len() < 1 || pkg_path.len() < 1 {
            println!("{}: {}", "publish error".red(), "not a valid name or didn't find any registries to publish to");
            return;
        }

        // Zip up the package directory so that it can be published
        let tmp_file_name = nanoid!();
        let _ = fs::create_dir_all("__stof_staging__");
        let path = format!("__stof_staging__/{}.zip", tmp_file_name);
        let file = fs::File::create(&path).unwrap();

        let walkdir = WalkDir::new(dir);
        let iter = walkdir.into_iter();
        let res = zip_directory(&mut iter.filter_map(|e| e.ok()), dir, file, zip::CompressionMethod::Bzip2, &excluded);
        if res.is_err() {
            println!("{}: {}", "publish error".red(), "failed to zip package directory".italic().dimmed());
            let _ = fs::remove_dir_all("__stof_staging__");
            return;
        }
        if let Ok(bytes) = fs::read(&path) {
            let pkg = Arc::new(Mutex::new((pkg_doc, Bytes::from(bytes))));
            let mut set = JoinSet::new();
            for reg in publish_registries {
                set.spawn(publish_to_registry(pkg.clone(), reg, pkg_path.clone()));
            }
            while let Some(_res) = set.join_next().await {
                // don't need anything here currently...
            }
        }
        let _ = fs::remove_dir_all("__stof_staging__");
        println!("{}", "publish success".green());
    } else {
        println!("{}: {}", "publish error".red(), "pkg.stof file not found".italic().dimmed());
    }
}


/// Zip the directory into an output file.
fn zip_directory<T: Write + Seek>(iter: &mut dyn Iterator<Item = DirEntry>, prefix: &str, writer: T, method: zip::CompressionMethod, excluded: &HashSet<String>) -> anyhow::Result<()> {
    let mut zip = zip::ZipWriter::new(writer);
    let options = SimpleFileOptions::default().compression_method(method).unix_permissions(0o755);

    let pref = Path::new(prefix);
    let mut buffer = Vec::new();
    'entries: for entry in iter {
        let path = entry.path();
        
        // don't add/publish any files that are in the reserved __stof__ directory
        let display = path.display().to_string();
        if display.contains("__stof__") {
            continue;
        }
        for exclude in excluded {
            if let Ok(re) = Regex::new(&exclude) {
                if re.is_match(&display) {
                    continue 'entries;
                }
            }
        }

        let name = path.strip_prefix(pref).unwrap();
        let path_as_string = name
            .to_str()
            .map(str::to_owned)
            .with_context(|| format!("{name:?} Is a Non UTF-8 Path"))?;

        if path.is_file() {
            zip.start_file(path_as_string, options)?;
            let mut f = File::open(path)?;

            f.read_to_end(&mut buffer)?;
            zip.write_all(&buffer)?;
            buffer.clear();
        } else if !name.as_os_str().is_empty() {
            zip.add_directory(path_as_string, options)?;
        }
    }
    zip.finish()?;
    Ok(())
}


/// Publish the package to a specific registry.
async fn publish_to_registry(pkg: Arc<Mutex<(SDoc, Bytes)>>, registry: SNodeRef, publish_path: String) {
    let mut url = String::default();
    let bytes;
    {
        let pkg = pkg.lock().await;
        let doc = &pkg.0;
        bytes = pkg.1.clone();

        if let Some(url_field) = SField::field(&doc.graph, "registry.url", '.', Some(&registry)) {
            url = url_field.to_string();
        }
    }

    if bytes.len() > 0 && url.len() > 0 {
        let url = format!("{}/registry/{}", url, publish_path);
        let client = reqwest::Client::new();
        let res = client.put(&url)
            .body(bytes)
            .send()
            .await;
        match res {
            Ok(resp) => {
                let text = resp.text().await.unwrap();
                println!("{} ... {}", url.blue(), text.italic().dimmed());
            },
            Err(error) => {
                println!("{}: {}", "publish send error".red(), error.to_string().italic().dimmed());
            }
        }
    } else {
        println!("{}: {}", "publish error".red(), "registry URL not found, or package has a size of 0 bytes".italic().dimmed());
    }
}


/// Unpublish a stof package to registries.
pub(crate) async fn unpublish_package(dir: &str) {
    let pkg_path = format!("{}/pkg.stof", dir);
    if let Ok(pkg_doc) = SDoc::file(&pkg_path, "stof") {
        let mut pkg_path = String::default();
        let mut publish_registries = Vec::new();

        if let Some(name_field) = SField::field(&pkg_doc.graph, "root.name", '.', None) {
            let pkg_name = name_field.to_string();
            pkg_path = pkg_name.trim_start_matches("@").to_owned();

            if let Some(publish_array) = SField::field(&pkg_doc.graph, "root.publish", '.', None) {
                match &publish_array.value {
                    SVal::Array(vals) => {
                        for val in vals {
                            match val {
                                SVal::Object(nref) => {
                                    publish_registries.push(nref.clone());
                                },
                                _ => {}
                            }
                        }
                    },
                    _ => {}
                }
            }
        }

        if publish_registries.len() < 1 || pkg_path.len() < 1 {
            println!("{}: {}", "unpublish error".red(), "not a valid name or didn't find any registries to unpublish from");
            return;
        }

        let client = reqwest::Client::new();
        for registry in publish_registries {
            if let Some(url_field) = SField::field(&pkg_doc.graph, "registry.url", '.', Some(&registry)) {
                let url = format!("{}/registry/{}", url_field.to_string(), &pkg_path);
                let res = client.delete(&url).send().await;
                match res {
                    Ok(response) => {
                        let text = response.text().await.unwrap();
                        println!("{} ... {}", url.blue(), text.italic().dimmed());
                    },
                    Err(error) => {
                        println!("{}: {}", "unpublish send error".red(), error.to_string().italic().dimmed());
                    },
                }
            }
        }
        println!("{}", "successfully removed package from all registries".green());
    } else {
        println!("{}: {}", "unpublish error".red(), "pkg.stof file not found".italic().dimmed());
    }
}
