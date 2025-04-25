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

use std::fs;
use colored::Colorize;
use http_auth_basic::Credentials;
use reqwest::header::{HeaderMap, AUTHORIZATION};
use stof::{pkg::PKG, SDoc, SField, SVal};


/// Remove a stof package from this workspace.
pub(crate) async fn remove_package(pkg_dir_path: &str) -> bool {
    let path = format!("__stof__/{}", pkg_dir_path.trim_start_matches("@"));
    fs::remove_dir_all(path).is_ok()
}


/// Publish a stof package to registries.
pub(crate) async fn add_package(pkg_dir: &str, download_pkg: &str, registry: Option<String>, dependency: bool, username: Option<String>, password: Option<String>) {
    let pkg_path = format!("{}/pkg.stof", pkg_dir);
    if let Ok(pkg_doc) = SDoc::file(&pkg_path, "stof") {
        let mut reg = None;
        if let Some(reg_name) = registry {
            let path = format!("root.registries.{}", reg_name);
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
            if let Some(nref) = pkg_doc.graph.node_ref("root/registries", None) {
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

                let mut headers = HeaderMap::new();
                if username.is_some() && password.is_some() {
                    let credentials = Credentials::new(&username.clone().unwrap(), &password.clone().unwrap());
                    headers.insert(AUTHORIZATION, credentials.as_http_header().parse().unwrap());
                }

                let res = client.get(&url)
                    .headers(headers)
                    .send()
                    .await;

                match res {
                    Ok(response) => {
                        if response.status().is_success() {
                            if let Ok(bytes) = response.bytes().await {
                                let pkg_format = PKG::default();
                                let outdir = pkg_format.unzip_pkg_bytes(download_pkg, &bytes);
                                add_dependencies(&outdir, pkg_dir, username, password).await;
                                
                                if dependency {
                                    println!("\t{} {} {}", "...".dimmed(), "added dependency".purple(), download_pkg.blue());
                                } else {
                                    println!("{} {}", "added".green(), download_pkg.blue());
                                }
                            } else {
                                println!("{}: {}", "publish send error".red(), "could not parse response into bytes".italic().dimmed());
                            }
                        } else {
                            println!("{}: {} {}", "publish send error".red(), download_pkg.blue(), "does not exist or not authenticated".italic().dimmed());
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


/// Add dependencies for the newly added package.
async fn add_dependencies(outdir: &str, pkg_dir: &str, username: Option<String>, password: Option<String>) {
    let added_pkg_path = format!("{}/pkg.stof", outdir);
    if let Ok(added_pkg_doc) = SDoc::file(&added_pkg_path, "stof") {
        if let Some(deps_field) = SField::field(&added_pkg_doc.graph, "root.dependencies", '.', None) {
            match &deps_field.value {
                SVal::Array(vals) => {
                    for val in vals {
                        match val {
                            SVal::String(download_pkg) => {
                                Box::pin(add_package(pkg_dir, download_pkg, None, true, username.clone(), password.clone())).await;
                            },
                            SVal::Object(nref) => {
                                if let Some(name_field) = SField::field(&added_pkg_doc.graph, "name", '.', Some(nref)) {
                                    if let Some(registry_field) = SField::field(&added_pkg_doc.graph, "registry", '.', Some(nref)) {
                                        Box::pin(add_package(pkg_dir, &name_field.to_string(), Some(registry_field.to_string()), true, username.clone(), password.clone())).await;
                                    } else {
                                        Box::pin(add_package(pkg_dir, &name_field.to_string(), None, true, username.clone(), password.clone())).await;
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
