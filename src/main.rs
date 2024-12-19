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

use clap::{Parser, Subcommand};
use stof::SDoc;

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
    },
    Test {
        /// File to test.
        file: String,
    }
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Run { file } => {
            let mut format = "stof";
            if file.ends_with("bstof") {
                format = "bstof";
            }
            let res = SDoc::file(&file, format);
            match res {
                Ok(mut doc) => {
                    doc.run(None);
                },
                Err(error) => {
                    eprintln!("stof error: was not able to load '{}' in the format '{}': {}", &file, &format, error.to_string());
                }
            }
        },
        Command::Test { file } => {
            SDoc::test_file(&file, false);
        },
    }
}
