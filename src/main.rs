//
// Copyright 2025 Formata, Inc. All rights reserved.
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

use std::{collections::HashSet, fs, ops::Deref, path::PathBuf};
use clap::{Parser, Subcommand};
use colog::format::CologStyle;
use colored::Colorize;
use log::Level;
use stof::{model::{Field, Graph, Profile, StofPackageFormat, prof::insert_profile_lib}, runtime::{Error, Runtime, Val}};


pub struct StofCliLogger;
impl CologStyle for StofCliLogger {
    fn level_token(&self, level: &log::Level) -> &str {
        match *level {
            Level::Error => "ERROR",
            Level::Warn => "WARN",
            Level::Info => "INFO",
            Level::Debug => "DEBUG",
            Level::Trace => "TRACE",
        }
    }
}


const BANNER: &str = r#"
   _____ __________  ______
  / ___//_  __/ __ \/ ____/
  \__ \  / / / / / / /_    
 ___/ / / / / /_/ / __/    
/____/ /_/  \____/_/
"#;


#[derive(Parser, Debug)]
#[command(author, version, about = "Run, test, document, and package Stof: portable, sandboxed logic inside data", long_about = None, before_help = BANNER)]
struct Cli {
    #[arg(short, long, action = clap::ArgAction::Count)]
    /// Turn debug logging on ("log_error" & "log_warn" always on) ("-d" for "log_info" logs, "-dd" for "log_trace" & "log_debug" also)
    debug: u8,

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
        #[arg(long)]
        /// Run tests one at a time and fail any test that leaves objects behind (created, never dropped, not in any field).
        leaks: bool,

        /// Path to a file or package to import.
        path: Option<String>,

        /// Context to test.
        context: Option<String>,
    },

    /// Create documentation for a file or package using the "docs" format.
    Docs {
        #[arg(long)]
        /// Include tests in docs?
        tests: bool,

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

    /// Unpackage a Stof package (.pkg) file into a directory of choice.
    Unpkg {
        /// Path to a Stof package (.pkg) file.
        path: String,

        /// Optional output directory (defaults to "stof/<PATH NAME>").
        out: Option<String>,
    },
}

/// Main.
fn main() {
    let cli = Cli::parse();
    let mut builder = env_logger::builder();
    builder.format(colog::formatter(StofCliLogger));
    match cli.debug {
        0 => builder.filter(None, log::LevelFilter::Warn),
        1 => builder.filter(None, log::LevelFilter::Info),
        _ => builder.filter(None, log::LevelFilter::Trace),
    };
    builder.init();

    match cli.command {
        Command::Run { path , mut attribute } => {
            let mut graph;
            if let Some(path) = path {
                if path == "." {
                    graph = create_graph("", GraphProfile::Prod);
                } else {
                    graph = create_graph(&path, GraphProfile::Prod);
                }
            } else {
                graph = create_graph("", GraphProfile::Prod);
            }

            if attribute.len() < 1 { attribute.push("main".into()); } // main functions by default
            let attributes = attribute
                .into_iter()
                .collect();
            
            match Runtime::run_attribute_functions(&mut graph, None, &Some(attributes), true) {
                Ok(res) => println!("{res}"),
                Err(res) => {
                    println!("{res}");
                    std::process::exit(1); // failures are visible to scripts and CI
                },
            }
        },
        Command::Test { leaks, path, context } => {
            let mut graph;
            if let Some(path) = path {
                if path == "." {
                    graph = create_graph("", GraphProfile::Test);
                } else {
                    graph = create_graph(&path, GraphProfile::Test);
                }
            } else {
                graph = create_graph("", GraphProfile::Test);
            }
            let res = if leaks { graph.test_leaks(context, true) } else { graph.test(context, true) };
            match res {
                Ok(res) => println!("{res}"),
                Err(res) => {
                    println!("{res}");
                    std::process::exit(1); // failing tests fail the command (CI)
                },
            }
        },
        Command::Docs { tests, path, out } => {
            let mut out_path = String::from("./");
            if let Some(out) = out {
                out_path = out;
            }

            let mut in_path = String::default();
            if let Some(path) = path {
                in_path = path;
            }

            let mut prof = GraphProfile::Docs;
            if tests { prof = GraphProfile::TestDocs; }
            let graph = create_graph(&in_path, prof);
            match graph.docs(&out_path, None) {
                Ok(_) => {
                    println!("{} {}", "created docs".green(), out_path.blue());
                },
                Err(error) => {
                    println!("{} {}", "docs creation error".red(), error.to_string());
                    std::process::exit(1);
                }
            }
        },
        Command::Pkg { path, out } => {
            let mut dir = ".".to_string();
            if let Some(path) = path {
                dir = path;
            }

            let mut out_path = format!("{dir}/out.pkg");
            if let Some(out) = out {
                out_path = out;
            }
            let mut included = HashSet::new();
            let mut excluded = HashSet::new();

            let pkg_path = format!("{dir}/pkg.stof");
            if let Ok(exists) = fs::exists(&pkg_path) {
                if exists {
                    let mut graph = Graph::default();
                    graph.allow_system(); // reading pkg.stof (and its imports) from disk
                    if let Err(error) = graph.file_import("stof", &pkg_path, None, &Profile::default()) {
                        log::error!("{} {}", "pkg.stof error:".red(), error.to_string());
                        std::process::exit(1);
                    }
                    let root = graph.ensure_main_root();

                    // Include files
                    if let Some(field) = Field::direct_field(&graph, &root, "include") {
                        if let Some(field) = graph.get_stof_data::<Field>(&field) {
                            match field.value.val.read().deref() {
                                Val::List(patterns) => {
                                    for pattern in patterns {
                                        match pattern.read().deref() {
                                            Val::Str(regex) => {
                                                included.insert(regex.to_string());
                                            },
                                            _ => {}
                                        }
                                    }
                                },
                                Val::Set(patterns) => {
                                    for pattern in patterns {
                                        match pattern.read().deref() {
                                            Val::Str(regex) => {
                                                included.insert(regex.to_string());
                                            },
                                            _ => {}
                                        }
                                    }
                                },
                                _ => {}
                            }
                        }
                    }

                    // Exclude files
                    if let Some(field) = Field::direct_field(&graph, &root, "exclude") {
                        if let Some(field) = graph.get_stof_data::<Field>(&field) {
                            match field.value.val.read().deref() {
                                Val::List(patterns) => {
                                    for pattern in patterns {
                                        match pattern.read().deref() {
                                            Val::Str(regex) => {
                                                excluded.insert(regex.to_string());
                                            },
                                            _ => {}
                                        }
                                    }
                                },
                                Val::Set(patterns) => {
                                    for pattern in patterns {
                                        match pattern.read().deref() {
                                            Val::Str(regex) => {
                                                excluded.insert(regex.to_string());
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
            }

            if !out_path.ends_with(".pkg") { out_path.push_str(".pkg"); }

            // Never pack the output file itself (the default output is inside the directory, and an
            // older package there would otherwise end up in the new one)
            if let (Ok(out_abs), Ok(dir_abs)) = (std::path::absolute(&out_path), std::path::absolute(&dir)) {
                if let Ok(relative) = out_abs.strip_prefix(&dir_abs) {
                    let relative = relative.to_string_lossy().replace('\\', "/");
                    excluded.insert(format!("^{}$", regex_escape(&relative)));
                }
            }

            // Build in a temp file first so the package being written isn't part of its own contents
            let temp_path = std::env::temp_dir().join(format!("stof-pkg-{}.pkg", std::process::id()));
            let Some(created) = StofPackageFormat::create_package_file(&dir, &temp_path.to_string_lossy(), &included, &excluded) else {
                log::error!("{}", "pkg creation error".red());
                std::process::exit(1);
            };
            if let Some(parent) = PathBuf::from(&out_path).parent() {
                if !parent.as_os_str().is_empty() { let _ = fs::create_dir_all(parent); }
            }
            let copied = fs::copy(&created, &out_path);
            let _ = fs::remove_file(&created);
            match copied {
                Ok(_) => println!("{} {}", "created".green(), out_path.blue()),
                Err(error) => {
                    log::error!("{} {}: {}", "pkg creation error".red(), out_path.blue(), error);
                    std::process::exit(1);
                }
            }
        },
        Command::Unpkg { mut path, out } => {
            if !path.contains('.') {
                path = format!("{path}.pkg");
            }
            let dir;
            if let Some(out) = out {
                dir = out;
            } else {
                let buf = PathBuf::from(&path);
                let mut stem = buf.file_stem().unwrap_or_default().to_str().unwrap_or_default().to_string();
                stem = stem.replace('.', "_");
                dir = format!("./stof/{stem}");
            }
            if !fs::exists(&path).unwrap_or(false) {
                log::error!("{} {}", "package not found:".red(), path.blue());
                std::process::exit(1);
            }
            let _ = fs::create_dir_all(&dir);

            StofPackageFormat::unzip_file(&path, &dir);
            println!("{} {}", "unpacked".green(), path.blue());
        },
    }
}


/// Escape regex metacharacters (package include/exclude patterns are regexes).
fn regex_escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(ch) { escaped.push('\\'); }
        escaped.push(ch);
    }
    escaped
}


enum GraphProfile {
    Test,
    TestDocs,
    Prod,
    Docs,
}


/// Create a stof graph from a file path.
fn create_graph(path: &str, prof: GraphProfile) -> Graph {
    let path_buf;
    if path.len() > 0 {
        path_buf = PathBuf::from(path);
    } else if let Ok(buf) = std::env::current_dir() {
        path_buf = buf;
    } else {
        panic!("{} {}: {}", "parse error".red(), path.blue(), "no directory or path found".dimmed());
    }
    
    let mut graph = Graph::default();
    graph.set_deadpools_enabled(false); // no need for deadpools with CLI

    // The CLI runs your own documents: give them the file system, environment, and network
    // (embedded Stof is sandboxed by default; hosts opt in to these)
    graph.allow_system();
    graph.allow_http();

    let profile = match prof {
        GraphProfile::Prod => {
            // statement locations in error messages (file:line:col); costs ~1-2% when running
            let mut profile = Profile::prod();
            profile.debug_info = true;
            profile
        },
        GraphProfile::Docs => Profile::docs(false),
        GraphProfile::Test => Profile::test(),
        GraphProfile::TestDocs => Profile::docs(true),
    };
    insert_profile_lib(&mut graph, &profile); // should be redundant, but just in case

    let res;
    if path_buf.is_dir() {
        res = graph.file_import("pkg", path_buf.to_str().unwrap(), None, &profile);
    } else if let Some(format) = path_buf.extension() {
        if let Some(format) = format.to_str() {
            res = graph.file_import(format, path_buf.to_str().unwrap(), None, &profile);
        } else {
            res = Err(Error::Custom("could not retrieve import format".into()));
        }
    } else {
        res = Err(Error::Custom("could not determine the import format (no file extension)".into()));
    }

    match res {
        Ok(_) => {
            graph
        },
        Err(error) => {
            log::error!("{}", error.to_string());
            std::process::exit(1); // nothing to run: don't continue with an empty document
        }
    }
}
