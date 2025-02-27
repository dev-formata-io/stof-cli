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

use std::{collections::HashSet, fs, path::PathBuf};
use bytes::Bytes;
use colored::Colorize;
use reqwest::header::{HeaderMap, CONTENT_TYPE};
use stof::{pkg::PKG, SData, SDoc, SFunc};


/// Execute a stof document or package remotely, parsing/creating it on the remote server.
pub async fn remote_exec(address: &str, path: &str) {
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

    let mut bytes = None;
    if path_buf.is_dir() {
        headers.insert(CONTENT_TYPE, "pkg".parse().unwrap());
        let pkg_format = PKG::default();
        if let Some(zip_path) = pkg_format.create_temp_zip(path_buf.to_str().unwrap(), &HashSet::new()) {
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
pub async fn remote_exec_doc(address: &str, doc: &SDoc) {
    let bytes = doc.export_bytes("main", "bstof", None).unwrap();
    let url = format!("{}/run", address);

    let client = reqwest::Client::new();
    let mut headers = HeaderMap::new();
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
