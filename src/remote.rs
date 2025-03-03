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

use std::{fs, path::PathBuf};
use bytes::Bytes;
use colored::Colorize;
use http_auth_basic::Credentials;
use reqwest::header::{HeaderMap, AUTHORIZATION, CONTENT_TYPE};
use stof::{SData, SDoc, SFunc};
use crate::publish::create_temp_pkg_zip;


/// Execute a stof document or package remotely, parsing/creating it on the remote server.
pub async fn remote_exec(address: &str, path: &str, username: Option<String>, password: Option<String>) {
    let path_buf;
    if path.len() > 0 {
        path_buf = PathBuf::from(path);
    } else if let Ok(buf) = std::env::current_dir() {
        path_buf = buf;
    } else {
        eprintln!("{} {}: {}", "remote package creation error".red(), path.blue(), "no directory or path found".dimmed());
        return;
    }

    let url = format!("{}/run", address);
    let client = reqwest::Client::new();
    let mut headers = HeaderMap::new();
    if username.is_some() && password.is_some() {
        let credentials = Credentials::new(&username.unwrap(), &password.unwrap());
        headers.insert(AUTHORIZATION, credentials.as_http_header().parse().unwrap());
    }

    let mut bytes = None;
    if path_buf.is_dir() {
        headers.insert(CONTENT_TYPE, "pkg".parse().unwrap());
        if let Some(zip_path) = create_temp_pkg_zip(path_buf.to_str().unwrap()).await {
            if let Ok(vec) = fs::read(&zip_path) {
                bytes = Some(Bytes::from(vec));
            }
            let _ = fs::remove_file(&zip_path);
        }
    } else {
        if let Some(ext) = path_buf.extension() {
            // use the file's extension as the format/type
            headers.insert(CONTENT_TYPE, ext.to_str().unwrap().parse().unwrap());
        }
        if let Ok(vec) = fs::read(&path_buf) {
            bytes = Some(Bytes::from(vec));
        }
    }

    if let Some(bytes) = bytes {
        let res = client.post(url)
            .headers(headers)
            .body(bytes)
            .send()
            .await;

        match res {
            Ok(response) => {
                let headers = response.headers();
                let mut content_type = String::from("bstof");
                if let Some(ctype) = headers.get(CONTENT_TYPE) {
                    content_type = ctype.to_str().unwrap().to_owned();
                }

                if let Ok(mut bytes) = response.bytes().await {
                    let mut doc = SDoc::default();
                    let res = doc.header_import("main", &content_type, &content_type, &mut bytes, "");
                    match res {
                        Ok(_) => {
                            // Check for textual responses, including errors
                            if content_type.contains("text") {
                                if let Some(text_field) = doc.field("text", None) {
                                    println!("{}", text_field.to_string());
                                }
                            }

                            // The document comes back, call all #[local] functions on the main root of the document
                            if let Some(main) = doc.graph.main_root() {
                                let mut to_call = Vec::new();
                                for dref in SFunc::func_refs(&doc.graph, &main) {
                                    if let Some(func) = SData::get::<SFunc>(&doc.graph, &dref) {
                                        if func.attributes.contains_key("local") {
                                            to_call.push(dref);
                                        }
                                    }
                                }
                                for dref in to_call {
                                    let _ = SFunc::call(&dref, "main", &mut doc, vec![], true);
                                }
                            }
                        },
                        Err(error) => {
                            eprintln!("{} bad response : {}", "remote exec error".red(), error.to_string(&doc.graph).dimmed());
                        }
                    }
                }
            },
            Err(error) => {
                eprintln!("{}: {}", "remote exec error".red(), error.to_string().dimmed());
            },
        }
    } else {
        eprintln!("{} {}: {}", "remote package creation error".red(), path.blue(), "package/file contents not found".dimmed());
    }
}


/// Execute a stof document remotely after it's already been parsed/created.
pub async fn remote_exec_doc(address: &str, doc: &SDoc, username: Option<String>, password: Option<String>) {
    let bytes = doc.export_bytes("main", "bstof", None).unwrap();
    let url = format!("{}/run", address);

    let client = reqwest::Client::new();
    let mut headers = HeaderMap::new();
    if username.is_some() && password.is_some() {
        let credentials = Credentials::new(&username.unwrap(), &password.unwrap());
        headers.insert(AUTHORIZATION, credentials.as_http_header().parse().unwrap());
    }
    headers.insert(CONTENT_TYPE, "application/bstof".parse().unwrap());

    let res = client.post(url)
        .headers(headers)
        .body(bytes)
        .send()
        .await;

    match res {
        Ok(response) => {
            let headers = response.headers();
            let mut content_type = String::from("bstof");
            if let Some(ctype) = headers.get(CONTENT_TYPE) {
                content_type = ctype.to_str().unwrap().to_owned();
            }

            if let Ok(mut bytes) = response.bytes().await {
                let mut doc = SDoc::default();
                let res = doc.header_import("main", &content_type, &content_type, &mut bytes, "");
                match res {
                    Ok(_) => {
                        // Check for textual responses, including errors
                        if content_type.contains("text") {
                            if let Some(text_field) = doc.field("text", None) {
                                println!("{}", text_field.to_string());
                            }
                        }

                        // The document comes back, call all #[local] functions on the main root of the document
                        if let Some(main) = doc.graph.main_root() {
                            let mut to_call = Vec::new();
                            for dref in SFunc::func_refs(&doc.graph, &main) {
                                if let Some(func) = SData::get::<SFunc>(&doc.graph, &dref) {
                                    if func.attributes.contains_key("local") {
                                        to_call.push(dref);
                                    }
                                }
                            }
                            for dref in to_call {
                                let _ = SFunc::call(&dref, "main", &mut doc, vec![], true);
                            }
                        }
                    },
                    Err(error) => {
                        eprintln!("{} bad response : {}", "remote exec error".red(), error.to_string(&doc.graph).dimmed());
                    }
                }
            }
        },
        Err(error) => {
            eprintln!("{}: {}", "remote exec error".red(), error.to_string().dimmed());
        },
    }
}

/// Set remote user.
/// Need admin permissions on the server, along with the user information to create/set.
/// Perms: 0b001 - read registry, 0b010 - modify registry, 0b100 - exec
/// Scope: optional, restricts modification of the registry to a specific top-level scope for a user. Ex. "formata" would allow modification to only @formata/... packages.
pub async fn set_remote_user(address: &str, admin_user: &str, admin_pass: &str, user: &str, pass: &str, perms: i64, scope: &str) {
    let url = format!("{}/admin/users", address);
    let payload = format!("username: '{}', password: '{}', perms: {}, scope: '{}'", user, pass, perms, scope);
    
    let mut headers = HeaderMap::new();
    let credentials = Credentials::new(admin_user, admin_pass);
    headers.insert(AUTHORIZATION, credentials.as_http_header().parse().unwrap());
    headers.insert(CONTENT_TYPE, "application/stof".parse().unwrap());

    let client = reqwest::Client::new();
    let res = client.post(url)
        .headers(headers)
        .body(payload)
        .send().await;

    match res {
        Ok(response) => {
            if let Ok(text) = response.text().await {
                println!("{}", text);
            } else {
                println!("success, but non-textual response");
            }
        },
        Err(error) => {
            eprintln!("{}", error.to_string().red());
        }
    }
}

/// Remove remote user.
/// Need admin permissions on the server, along with the username to delete.
pub async fn remove_remote_user(address: &str, admin_user: &str, admin_pass: &str, user: &str) {
    let url = format!("{}/admin/users", address);
    let payload = format!("username: '{}'", user);
    
    let mut headers = HeaderMap::new();
    let credentials = Credentials::new(admin_user, admin_pass);
    headers.insert(AUTHORIZATION, credentials.as_http_header().parse().unwrap());
    headers.insert(CONTENT_TYPE, "application/stof".parse().unwrap());

    let client = reqwest::Client::new();
    let res = client.delete(url)
        .headers(headers)
        .body(payload)
        .send().await;

    match res {
        Ok(response) => {
            if let Ok(text) = response.text().await {
                println!("{}", text);
            } else {
                println!("success, but non-textual response");
            }
        },
        Err(error) => {
            eprintln!("{}", error.to_string().red());
        }
    }
}
