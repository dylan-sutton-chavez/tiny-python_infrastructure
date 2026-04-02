#![no_std]
extern crate alloc; // Enables heap allocation without the standard library.

/* 
Webassembly architecture entry point.
*/

#[cfg(target_arch = "wasm32")]
pub mod wasm;

/*
Internal modules accessed through all the package.
*/

pub mod modules {
    pub mod lexer;
    pub mod parser;
    pub mod vm;
}