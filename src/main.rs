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
use publish::{create_pkg_zip, publish_package, unpublish_package};

mod add;
use add::{add_package, remove_package};

mod remote;
use remote::{remote_exec, remote_exec_doc};

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
    /// Run a file or package, calling all #[main] functions.
    /// Optionally, this command can be used to run stof remotely.
    Run {
        /// Path to file or package directory to run.
        path: Option<String>,

        /// Run on a remote server.
        #[arg(short, long, value_name = "ADDRESS")]
        on: Option<String>,

        /// When running on a remote server, should the file/package be parsed locally?
        #[arg(short = 'l', long)]
        parse_local: bool,

        /// Library allow list. Ex. "http" enables the HTTP library.
        #[arg(short, long)]
        allow: Vec<String>,
    },

    /// Test a file or package, running all #[test] functions.
    Test {
        /// Path to file or package directory to test.
        path: Option<String>,

        /// Library allow list. Ex. "http" enables the HTTP library.
        #[arg(short, long)]
        allow: Vec<String>,
    },

    /// Serve a file or package using the HTTP library server.
    Serve {
        /// Path to file or package directory to serve.
        path: Option<String>,

        /// Library allow list. Ex. "http" enables the HTTP library.
        #[arg(short, long)]
        allow: Vec<String>,
    },

    /// Publish this package to each registry defined in the pkg.stof publish array.
    Publish {
        /// Package directory, containing pkg.stof file.
        /// Default is to use the current working directory.
        dir: Option<String>,
    },

    /// Unpublish this package from each registry defined in the pkg.stof publish array.
    Unpublish {
        /// Package directory, containing pkg.stof file.
        /// Default is to use the current working directory.
        dir: Option<String>,
    },

    /// Add a remote package to this workspace, placed within the __stof__ directory for import access via "@path" syntax.
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

    /// Remove a package from this workspace.
    Remove {
        /// Package to remove.
        package: String,
    },

    /// Create a package file (.pkg) from a directory that contains a pkg.stof file.
    Pkg {
        /// Package directory (containing pkg.stof file) to turn into a package file (.pkg).
        dir: String,

        /// Optional output file path (.pkg).
        /// Default is <DIR>.pkg.
        out: Option<String>,
    },
}


#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Run { path, on, allow, parse_local } => {
            // Execute the entire run command remotely if requested
            if !parse_local && on.is_some() {
                if let Some(remote_address) = on {
                    if let Some(path) = path {
                        remote_exec(&remote_address, &path).await;
                    } else {
                        remote_exec(&remote_address, "").await;
                    }
                    return;
                }
            }

            // Create the document that will be ran
            let mut doc;
            if let Some(path) = path {
                doc = create_doc(&path, &allow);
            } else {
                doc = create_doc("", &allow);
            }

            // Execute this document remotely
            if let Some(remote_address) = on {
                remote_exec_doc(&remote_address, &doc).await;
                return;
            }

            // Run the document locally
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
            add_package(&pkg_dir, &package, registry, false).await;
        },
        Command::Remove { package } => {
            if remove_package(&package).await {
                println!("{} {}", "removed".green(), package.blue());
            } else {
                println!("{} {}", "failed to remove".red(), package.blue());
            }
        },
        Command::Pkg { dir, out } => {
            let mut out_path = dir.clone();
            if let Some(out) = out {
                out_path = out;
            }
            if let Some(path) = create_pkg_zip(&dir, &out_path, true).await {
                println!("{} {}", "created".green(), path.blue());
            } else {
                println!("{}", "pkg creation error".red());
            }
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
            res = doc.file_import("main", format, path_buf.to_str().unwrap(), format, "");
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
