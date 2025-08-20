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

use std::{collections::HashSet, path::PathBuf};
use clap::{Parser, Subcommand};
use colored::Colorize;
use stof::{model::{Graph, StofPackageFormat}, runtime::{Error, Runtime}};


#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}


#[derive(Subcommand, Debug)]
enum Command {
    /// Run a file or package, calling all #[main] functions.
    Run {
        /// Path to a file or package to import.
        path: Option<String>,

        /// Optional function attributes to run instead of #[main].
        #[arg(short, long)]
        attribute: Vec<String>,
    },

    /// Test a file or package, running all #[test] functions.
    Test {
        /// Path to a file or package to import.
        path: Option<String>,

        /// Context to test.
        context: Option<String>,
    },

    /// Create documentation for a file or package using the "docs" format.
    Docs {
        /// Path to a directory or file to import.
        path: Option<String>,

        /// Optional document output directory.
        out: Option<String>,
    },

    /// Create a package file (.pkg) from a directory that contains a pkg.stof file.
    Pkg {
        /// Path to a directory (with a pkg.stof file).
        path: Option<String>,

        /// Optional output file path (.pkg).
        /// Default is <PATH>/out.pkg.
        out: Option<String>,
    },
}

/// Main.
fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Run { path , mut attribute } => {
            let mut graph;
            if let Some(path) = path {
                graph = create_graph(&path);
            } else {
                graph = create_graph("");
            }

            if attribute.len() < 1 { attribute.push("main".into()); } // main funtions by default
            let attributes = attribute
                .into_iter()
                .collect();
            
            match Runtime::run_attribute_functions(&mut graph, None, &Some(attributes), true) {
                Ok(res) => println!("{res}"),
                Err(res) => println!("{res}"),
            }
        },
        Command::Test { path, context } => {
            let mut graph;
            if let Some(path) = path {
                graph = create_graph(&path);
            } else {
                graph = create_graph("");
            }
            match graph.test(context, true) {
                Ok(res) => println!("{res}"),
                Err(res) => println!("{res}"),
            }
        },
        Command::Docs { path, out } => {
            let mut out_path = String::from("./");
            if let Some(out) = out {
                out_path = out;
            }

            let mut in_path = String::default();
            if let Some(path) = path {
                in_path = path;
            }

            let graph = create_graph(&in_path);
            match graph.docs(&out_path, None) {
                Ok(_) => {
                    println!("{} {}", "created docs".green(), out_path.blue());
                },
                Err(error) => {
                    println!("{} {}", "docs creation error".red(), error.to_string());
                }
            }
        },
        Command::Pkg { path, out } => {
            let mut dir = "".to_string();
            if let Some(path) = path {
                dir = path;
            }

            let mut out_path = format!("{dir}/out.pkg");
            if let Some(out) = out {
                out_path = out;
            }
            let included = HashSet::new();
            let excluded = HashSet::new();
            if let Some(path) = StofPackageFormat::create_package_file(&dir, &out_path, &included, &excluded) {
                println!("{} {}", "created".green(), path.blue());
            } else {
                println!("{}", "pkg creation error".red());
            }
        },
    }
}


/// Create a stof graph from a file path.
fn create_graph(path: &str) -> Graph {
    let path_buf;
    if path.len() > 0 {
        path_buf = PathBuf::from(path);
    } else if let Ok(buf) = std::env::current_dir() {
        path_buf = buf;
    } else {
        panic!("{} {}: {}", "parse error".red(), path.blue(), "no directory or path found".dimmed());
    }
    
    let mut graph = Graph::default();

    let res;
    if path_buf.is_dir() {
        res = graph.file_import("pkg", path_buf.to_str().unwrap(), None);
    } else if let Some(format) = path_buf.extension() {
        if let Some(format) = format.to_str() {
            res = graph.file_import(format, path_buf.to_str().unwrap(), None);
        } else {
            res = Err(Error::Custom("could not retrieve import format".into()));
        }
    } else {
        res = Err(Error::Custom("could not determin import extension".into()));
    }

    match res {
        Ok(_) => {
            graph
        },
        Err(error) => {
            eprintln!("{}", error.to_string());
            Graph::default()
        }
    }
}
