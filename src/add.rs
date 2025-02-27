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

use std::{fs, io, path::PathBuf};
use bytes::Bytes;
use colored::Colorize;
use stof::{SDoc, SField, SVal};


/// Publish a stof package to registries.
pub(crate) async fn add_package(pkg_dir: &str, download_pkg: &str, registry: Option<String>) {
    let pkg_path = format!("{}/pkg.stof", pkg_dir);
    if let Ok(pkg_doc) = SDoc::file(&pkg_path, "stof") {
        let mut reg = None;
        if let Some(reg_name) = registry {
            let path = format!("root.add.{}", reg_name);
            if let Some(field) = SField::field(&pkg_doc.graph, &path, '.', None) {
                match &field.value {
                    SVal::Object(nref) => {
                        reg = Some(nref.clone());
                    },
                    _ => {}
                }
            }
        } else {
            // look for default registry (or first one present)
            if let Some(nref) = pkg_doc.graph.node_ref("root/add", None) {
                for field in SField::fields(&pkg_doc.graph, &nref) {
                    match &field.value {
                        SVal::Object(nref) => {
                            if reg.is_none() {
                                reg = Some(nref.clone());
                            } else if field.attributes.contains_key("default") {
                                reg = Some(nref.clone());
                            }
                        },
                        _ => {}
                    }
                }
            }
        }

        if let Some(registry) = reg {
            if let Some(url_field) = SField::field(&pkg_doc.graph, "registry.url", '.', Some(&registry)) {
                let download = download_pkg.trim_start_matches("@").to_owned();
                let url = format!("{}/registry/{}", url_field.to_string(), download);
                let client = reqwest::Client::new();
                let res = client.get(&url).send().await;
                match res {
                    Ok(response) => {
                        if let Ok(bytes) = response.bytes().await {
                            unzip_package(pkg_dir, download_pkg, bytes).await;
                            println!("{}", "successfully added package".green());
                        } else {
                            println!("{}: {}", "publish send error".red(), "could not parse response into bytes".italic().dimmed());
                        }
                    },
                    Err(error) => {
                        println!("{}: {}", "add send error".red(), error.to_string().italic().dimmed());
                    },
                }
            } else {
                println!("{}: {}", "add package error".red(), "registry URL not found".italic().dimmed());
            }
        } else {
            println!("{}: {}", "add package error".red(), "registry not found - make sure one is defined in your 'pkg.stof' file".italic().dimmed());
        }
    } else {
        println!("{}: {}", "add package error".red(), "pkg.stof file not found".italic().dimmed());
    }
}


/// Unzip package into __stof__ directory.
async fn unzip_package(pkg_dir: &str, download_pkg: &str, bytes: Bytes) {
    let outdir = format!("{}/__stof__/{}", pkg_dir, download_pkg.trim_start_matches("@"));
    let _ = fs::remove_dir_all(&outdir); // remove all contents of the existing out dir if any
    let _ = fs::create_dir_all(&outdir); // make sure the out dir exists

    // Load bytes into a temp zip file (TODO: don't do this)
    let _ = fs::create_dir_all("__stof_staging__");
    let _ = fs::write("__stof_staging__/tmp.zip", bytes);

    if let Ok(file) = fs::File::open("__stof_staging__/tmp.zip") {
        if let Ok(mut archive) = zip::ZipArchive::new(file) {
            for i in 0..archive.len() {
                let mut file = archive.by_index(i).unwrap();
                
                let outname = match file.enclosed_name() {
                    Some(path) => path,
                    None => continue,
                };
                
                let mut outpath = PathBuf::from(&outdir);
                outpath.push(outname);
                
                if file.is_dir() {
                    let _ = fs::create_dir_all(&outpath);
                } else {
                    if let Some(p) = outpath.parent() {
                        if !p.exists() {
                            let _ = fs::create_dir_all(p);
                        }
                    }
                    if let Ok(mut outfile) = fs::File::create(&outpath) {
                        let _ = io::copy(&mut file, &mut outfile);
                    }
                }
            }
        }
    }
    let _ = fs::remove_dir_all("__stof_staging__");
    add_dependencies(&outdir, pkg_dir).await;
}


/// Add dependencies for the newly added package.
async fn add_dependencies(outdir: &str, pkg_dir: &str) {
    let added_pkg_path = format!("{}/pkg.stof", outdir);
    if let Ok(added_pkg_doc) = SDoc::file(&added_pkg_path, "stof") {
        if let Some(deps_field) = SField::field(&added_pkg_doc.graph, "root.dependencies", '.', None) {
            match &deps_field.value {
                SVal::Array(vals) => {
                    for val in vals {
                        match val {
                            SVal::String(download_pkg) => {
                                Box::pin(add_package(pkg_dir, download_pkg, None)).await;
                            },
                            SVal::Object(nref) => {
                                if let Some(name_field) = SField::field(&added_pkg_doc.graph, "name", '.', Some(nref)) {
                                    if let Some(registry_field) = SField::field(&added_pkg_doc.graph, "registry", '.', Some(nref)) {
                                        Box::pin(add_package(pkg_dir, &name_field.to_string(), Some(registry_field.to_string()))).await;
                                    } else {
                                        Box::pin(add_package(pkg_dir, &name_field.to_string(), None)).await;
                                    }
                                }
                            },
                            _ => {}
                        }
                    }
                },
                _ => {}
            }
        }
    }
}
