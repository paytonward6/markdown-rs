use std::fs;
use std::path::PathBuf;
use markdown::*;

fn main() {
    let filename = PathBuf::from("./test.md");
    let file = fs::read_to_string(filename).expect("Unablet to read  from file!");
    println!("{}", to_html(&file));
}
