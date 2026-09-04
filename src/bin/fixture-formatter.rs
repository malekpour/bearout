// SPDX-License-Identifier: Apache-2.0

//! A deterministic stand-in for an external formatter, used by the test
//! suite and built only with the `fixture-formatter` feature. It reads
//! standard input and behaves as its first argument says:
//! `echo` copies, `upper` uppercases, `fail` exits 3 with a message,
//! `sleep <seconds>` waits before echoing, `spew <bytes>` writes that much
//! output, `stderr <bytes>` writes that much to standard error then
//! echoes, `name <text>` replaces the first line with `name=<text>`,
//! `config <file>` replaces the first line with the file read from the
//! working directory, `cache` writes a cache file into the working
//! directory then echoes, `env` prints the color-related environment,
//! `edit <file>` overwrites that file while uppercasing, and `crash`
//! aborts.

use std::io::{Read, Write};

/// The input without its first line.
fn rest_of(input: &[u8]) -> &[u8] {
    input
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(&[][..], |at| &input[at + 1..])
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut input = Vec::new();
    std::io::stdin()
        .read_to_end(&mut input)
        .expect("read standard input");
    let mut out = std::io::stdout().lock();
    match args.first().map(String::as_str) {
        Some("upper") => out.write_all(&input.to_ascii_uppercase()),
        Some("fail") => {
            eprintln!("formatter says no\nand a second line");
            std::process::exit(3);
        }
        Some("sleep") => {
            let seconds: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
            std::thread::sleep(std::time::Duration::from_secs(seconds));
            out.write_all(&input)
        }
        Some("spew") => {
            let bytes: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
            let chunk = vec![b'x'; 65_536];
            let mut written = 0;
            while written < bytes {
                let take = chunk.len().min(bytes - written);
                if out.write_all(&chunk[..take]).is_err() {
                    break;
                }
                written += take;
            }
            Ok(())
        }
        Some("stderr") => {
            let bytes: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
            let mut err = std::io::stderr().lock();
            let _ = err.write_all(&vec![b'e'; bytes]);
            out.write_all(&input)
        }
        Some("name") => {
            let text = args.get(1).cloned().unwrap_or_default();
            out.write_all(format!("name={text}\n").as_bytes())
                .and_then(|()| out.write_all(rest_of(&input)))
        }
        Some("config") => {
            let file = args.get(1).cloned().unwrap_or_default();
            let config = std::fs::read(&file).unwrap_or_else(|_| b"<no config>\n".to_vec());
            out.write_all(&config)
                .and_then(|()| out.write_all(rest_of(&input)))
        }
        Some("cache") => {
            std::fs::write(".fixture-cache", b"cached").expect("write cache");
            let temp = std::env::var("TMPDIR").unwrap_or_default();
            if !temp.is_empty() {
                let _ = std::fs::write(std::path::Path::new(&temp).join("scratch"), b"tmp");
            }
            out.write_all(&input)
        }
        Some("env") => {
            let no_color = std::env::var("NO_COLOR").unwrap_or_default();
            let term = std::env::var("TERM").unwrap_or_default();
            out.write_all(format!("NO_COLOR={no_color} TERM={term}\n").as_bytes())
        }
        Some("edit") => {
            // Overwrite the named file while "formatting" it, to stand in
            // for a concurrent editor.
            let file = args.get(1).cloned().unwrap_or_default();
            std::fs::write(&file, b"edited meanwhile\n").expect("edit target");
            out.write_all(&input.to_ascii_uppercase())
        }
        Some("crash") => std::process::abort(),
        _ => out.write_all(&input),
    }
    .expect("write standard output");
}
