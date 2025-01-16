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

use std::sync::Arc;
use clap::{Parser, Subcommand};
use colored::Colorize;
use stof::SDoc;
use stof_github::{GitHubFormat, GitHubLibrary};
use stof_http::HTTPLibrary;


#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}


#[derive(Subcommand, Debug)]
enum Command {
    Run {
        /// File to run.
        file: String,

        /// Allow list.
        #[arg(short, long)]
        allow: Vec<String>,
    },
    Test {
        /// File to test.
        file: String,

        /// Allow list.
        #[arg(short, long)]
        allow: Vec<String>,
    }
}


fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Run { file, allow } => {
            let mut doc = create_doc(&file, &allow);
            let res = doc.run(None);
            match res {
                Ok(_) => {
                    // Nothing to do here...
                },
                Err(res) => println!("{res}"),
            }
        },
        Command::Test { file, allow } => {
            let mut doc = create_doc(&file, &allow);
            let res = doc.run_tests(false, None);
            match res {
                Ok(res) => println!("{res}"),
                Err(res) => println!("{res}"),
            }
        },
    }
}


/// Create a stof document from a file path.
fn create_doc(path: &str, allow: &Vec<String>) -> SDoc {
    let path_split = path.split('.').collect::<Vec<&str>>();
    let format = *path_split.last().unwrap();
    
    let mut doc = SDoc::default();
    allow_libs(&mut doc, allow);

    let res = doc.file_import("main", format, path, format, "");
    match res {
        Ok(_) => {
            doc
        },
        Err(error) => {
            eprintln!("{} {}: {}", "parse error".red(), path.blue(), error.to_string().dimmed());
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
