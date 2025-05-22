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
use remote::{remote_exec, remote_exec_doc, remove_remote_user, set_remote_user};

use std::path::PathBuf;
use clap::{Parser, Subcommand};
use colored::Colorize;
use stof::{lang::SError, SDoc};


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

        /// Full remote command?
        #[arg(short, long)]
        full_remote: bool,

        /// When running on a remote server, should the file/package be parsed locally?
        #[arg(short = 'l', long)]
        parse_local: bool,

        /// Library allow list. Ex. "http" enables the HTTP library.
        #[arg(short, long)]
        allow: Vec<String>,

        /// Optional remote username.
        #[arg(short, long)]
        username: Option<String>,

        /// Optional remote password.
        #[arg(short, long)]
        password: Option<String>,
    },

    /// Test a file or package, running all #[test] functions.
    Test {
        /// Path to file or package directory to test.
        path: Option<String>,

        /// Library allow list. Ex. "http" enables the HTTP library.
        #[arg(short, long)]
        allow: Vec<String>,
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

    /// Publish this package to each registry defined in the pkg.stof publish array.
    Publish {
        /// Package directory, containing pkg.stof file.
        /// Default is to use the current working directory.
        dir: Option<String>,

        /// Registry name.
        /// Registry with a #[default] attribute is used by default.
        #[arg(short, long)]
        registry: Option<String>,

        /// Optional remote username.
        #[arg(short, long)]
        username: Option<String>,

        /// Optional remote password.
        #[arg(short, long)]
        password: Option<String>,
    },

    /// Unpublish this package from each registry defined in the pkg.stof publish array.
    Unpublish {
        /// Package directory, containing pkg.stof file.
        /// Default is to use the current working directory.
        dir: Option<String>,

        /// Registry name.
        /// Registry with a #[default] attribute is used by default.
        #[arg(short, long)]
        registry: Option<String>,

        /// Optional remote username.
        #[arg(short, long)]
        username: Option<String>,

        /// Optional remote password.
        #[arg(short, long)]
        password: Option<String>,
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

        /// Optional remote username.
        #[arg(short, long)]
        username: Option<String>,

        /// Optional remote password.
        #[arg(short, long)]
        password: Option<String>,
    },

    /// Create or update a user on a specific runner.
    SetRemoteUser {
        /// Remote server address.
        server: String,

        /// Admin username.
        admin_user: String,

        /// Admin password.
        admin_pass: String,

        /// New user username.
        username: String,

        /// New user password.
        password: String,

        /// Permissions (0b1001 -> exec, delete, write, read).
        /// Default is 9, exec + read.
        #[arg(short, long)]
        perms: Option<i64>,

        /// Scope for this user, restricting the modification (write or delete) paths this user has access to.
        /// For example, a scope of "example" would allow this user (if permitted) to write and delte only the registry paths that start with "@example/".
        #[arg(short, long)]
        scope: Option<String>,
    },

    /// Remove a user on a specific runner.
    DeleteRemoteUser {
        /// Remote server address.
        server: String,

        /// Admin username.
        admin_user: String,

        /// Admin password.
        admin_pass: String,

        /// Username to remove.
        username: String,
    }
}


#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Run { path, on, full_remote, allow, parse_local, username, password } => {
            // Execute the entire run command remotely if requested
            if !parse_local && on.is_some() {
                if let Some(remote_address) = on {
                    if let Some(path) = path {
                        remote_exec(&remote_address, full_remote, &path, username, password).await;
                    } else {
                        remote_exec(&remote_address, full_remote, "", username, password).await;
                    }
                    return;
                }
            }

            // Create the document that will be ran
            let doc;
            if let Some(path) = path {
                doc = create_doc(&path, &allow);
            } else {
                doc = create_doc("", &allow);
            }

            // Execute this document remotely
            if let Some(remote_address) = on {
                remote_exec_doc(&remote_address, full_remote, &doc, username, password).await;
                return;
            }

            // Run the document locally
            let res = SDoc::run_blocking_async(doc, None, None).await;
            match res {
                Ok(_) => {
                    // Nothing to do here...
                },
                Err(res) => println!("{res}"),
            }
        },
        Command::Test { path, allow } => {
            let doc;
            if let Some(path) = path {
                doc = create_doc(&path, &allow);
            } else {
                doc = create_doc("", &allow);
            }

            let res = SDoc::test_blocking_async(doc, false, None).await;
            match res {
                Ok(res) => println!("{res}"),
                Err(res) => println!("{res}"),
            }
        },
        Command::Publish { dir, registry, username, password } => {
            let mut pkg_dir = "./".to_string();
            if let Some(dir) = dir {
                pkg_dir = dir;
            }
            publish_package(&pkg_dir, registry, username, password).await;
        },
        Command::Unpublish { dir, registry, username, password } => {
            let mut pkg_dir = "./".to_string();
            if let Some(dir) = dir {
                pkg_dir = dir;
            }
            unpublish_package(&pkg_dir, registry, username, password).await;
        },
        Command::Add { dir, registry, package, username, password } => {
            let mut pkg_dir = "./".to_string();
            if let Some(dir) = dir {
                pkg_dir = dir;
            }
            add_package(&pkg_dir, &package, registry, false, username, password).await;
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
        Command::SetRemoteUser { server, admin_user, admin_pass, username, password, perms, scope } => {
            let mut user_perms: i64 = 0b1001; // read and exec only access by default
            if let Some(prm) = perms {
                user_perms = prm;
            }
            let mut user_scope = String::default();
            if let Some(sc) = scope {
                user_scope = sc;
            }
            set_remote_user(&server, &admin_user, &admin_pass, &username, &password, user_perms, &user_scope).await;
        },
        Command::DeleteRemoteUser { server, admin_user, admin_pass, username } => {
            remove_remote_user(&server, &admin_user, &admin_pass, &username).await;
        }
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
    let mut http_enabled = false;
    for name in allow {
        match name.as_str() {
            "all" => {
                http_enabled = true;
            },
            "http" => {
                http_enabled = true;
            },
            _ => {
                println!("{}: {}", "unrecognized library".italic().dimmed(), name.purple());
            }
        }
    }
    if !http_enabled {
        doc.remove_library("Http");
    }
}
