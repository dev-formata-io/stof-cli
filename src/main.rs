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

mod publish;
use publish::{publish_package, unpublish_package};

mod add;
use add::add_package;

use std::{path::PathBuf, sync::Arc};
use clap::{Parser, Subcommand};
use colored::Colorize;
use stof::{lang::SError, SDoc};
use stof_github::{GitHubFormat, GitHubLibrary};
use stof_http::{server::serve, HTTPLibrary};


#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}


#[derive(Subcommand, Debug)]
enum Command {
    Run {
        /// Path to file or package directory to run.
        path: Option<String>,

        /// Allow list.
        #[arg(short, long)]
        allow: Vec<String>,
    },
    Test {
        /// Path to file or package directory to test.
        path: Option<String>,

        /// Allow list.
        #[arg(short, long)]
        allow: Vec<String>,
    },
    Serve {
        /// Path to file or package directory to serve.
        path: Option<String>,

        /// Allow list.
        #[arg(short, long)]
        allow: Vec<String>,
    },
    Publish {
        /// Package directory, containing pkg.stof file.
        /// Default is to use the current working directory.
        dir: Option<String>,
    },
    Unpublish {
        /// Package directory, containing pkg.stof file.
        /// Default is to use the current working directory.
        dir: Option<String>,
    },
    Add {
        /// Package to add.
        package: String,

        /// Package directory, containing pkg.stof file.
        /// Default is to use the current working directory.
        dir: Option<String>,

        /// Registry name.
        /// Registry with a #[default] attribute is used by default.
        #[arg(short, long)]
        registry: Option<String>,
    },
}


#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Run { path, allow } => {
            let mut doc;
            if let Some(path) = path {
                doc = create_doc(&path, &allow);
            } else {
                doc = create_doc("", &allow);
            }

            let res = doc.run(None);
            match res {
                Ok(_) => {
                    // Nothing to do here...
                },
                Err(res) => println!("{res}"),
            }
        },
        Command::Test { path, allow } => {
            let mut doc;
            if let Some(path) = path {
                doc = create_doc(&path, &allow);
            } else {
                doc = create_doc("", &allow);
            }

            let res = doc.run_tests(false, None);
            match res {
                Ok(res) => println!("{res}"),
                Err(res) => println!("{res}"),
            }
        },
        Command::Serve { path, allow } => {
            let doc;
            if let Some(path) = path {
                doc = create_doc(&path, &allow);
            } else {
                doc = create_doc("", &allow);
            }

            serve(doc); // start HTTP server with this document
        },
        Command::Publish { dir } => {
            let mut pkg_dir = "./".to_string();
            if let Some(dir) = dir {
                pkg_dir = dir;
            }
            publish_package(&pkg_dir).await;
        },
        Command::Unpublish { dir } => {
            let mut pkg_dir = "./".to_string();
            if let Some(dir) = dir {
                pkg_dir = dir;
            }
            unpublish_package(&pkg_dir).await;
        },
        Command::Add { dir, registry, package } => {
            let mut pkg_dir = "./".to_string();
            if let Some(dir) = dir {
                pkg_dir = dir;
            }
            add_package(&pkg_dir, &package, registry).await;
        },
    }
}


/// Create a stof document from a file path.
fn create_doc(path: &str, allow: &Vec<String>) -> SDoc {
    let path_buf;
    if path.len() > 0 {
        path_buf = PathBuf::from(path);
    } else if let Ok(buf) = std::env::current_dir() {
        path_buf = buf;
    } else {
        panic!("{} {}: {}", "parse error".red(), path.blue(), "no directory or path found".dimmed());
    }
    
    let mut doc = SDoc::default();
    allow_libs(&mut doc, allow);

    let res;
    if path_buf.is_dir() {
        res = doc.file_import("main", "pkg", path_buf.to_str().unwrap(), "stof", "");
    } else if let Some(format) = path_buf.extension() {
        if let Some(format) = format.to_str() {
            // If trying to create a doc from a zip pkg file, use the pkg format
            let mut import_format = format.to_owned();
            if format == "zip" { import_format = "pkg".to_owned(); }

            res = doc.file_import("main", &import_format, path_buf.to_str().unwrap(), format, "");
        } else {
            res = Err(SError::custom("main", &doc, "FormatError", "could not retrieve import format"));
        }
    } else {
        res = Err(SError::custom("main", &doc, "FormatError", "could not determin import extension"));
    }

    match res {
        Ok(_) => {
            doc
        },
        Err(error) => {
            eprintln!("{} {}: {}", "parse error".red(), path.blue(), error.message.dimmed());
            SDoc::default()
        }
    }
}


/// Allow libraries.
/// Because this is the CLI, the File System is enabled by default.
/// stof --allow all FILE_PATH
fn allow_libs(doc: &mut SDoc, allow: &Vec<String>) {
    for name in allow {
        match name.as_str() {
            "all" => {
                doc.load_lib(Arc::new(HTTPLibrary::default()));

                // Enables users to access their own GitHub repositories to add interfaces and data
                doc.load_lib(Arc::new(GitHubLibrary::default()));

                // Add default access to Formata's interfaces - this will change when we have a package manager...
                let mut formata = GitHubFormat::new("stof-formata", "dev-formata-io");
                formata.repo_id = "formata".into();
                doc.load_format(Arc::new(formata));
            },
            "http" => {
                doc.load_lib(Arc::new(HTTPLibrary::default()));
            },
            "github" => {
                // Enables users to access their own GitHub repositories to add interfaces and data
                doc.load_lib(Arc::new(GitHubLibrary::default()));

                // Add default access to Formata's interfaces - this will change when we have a package manager...
                let mut formata = GitHubFormat::new("stof-formata", "dev-formata-io");
                formata.repo_id = "formata".into();
                doc.load_format(Arc::new(formata));
            },
            _ => {
                println!("{}: {}", "unrecognized library".italic().dimmed(), name.purple());
            }
        }
    }
}
