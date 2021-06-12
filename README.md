# solana_rbpf (gdb)

Simple poc for attaching gdb to rbpf vm (tested on Ubuntu 18.04). This is very experimental and not all gdb features are supported (see 6). Most code architecture was already done here (see branches) https://github.com/Sladuca/rbpf/tree/main and described here https://github.com/solana-labs/solana/issues/14756
Only gdb needs to be downloaded and compiled, the other files are in tests/elfs.
We are using the file tests/elfs/test_simple_add.c for debugging (only 12 instructions)
1. Compile gdb with bpf target support (https://twitter.com/qeole/status/1291026052953911296
):
    - git clone git://sourceware.org/git/binutils-gdb.git
    - cd binutils-gdb
    - ./configure bpf
    - make    
(2+3 can be skipped, the files are already in tests/elfs directory)
2. Compile tests/elfs/test_simple_add.c for gdb usage:
    - cd tests/elfs
    - clang-12 -O2 -g -emit-llvm -c test_simple_add.c -o - | llc-12 -march=bpf -filetype=obj -o test_simple_add_gdb.o
     - strip conflicting elf sections with ./strip_elf.sh (using llvm-objcopy-12)
 3. Compile tests/elfs/test_simple_add.c for vm usage:
     The following paths do not exist in this repo. I cannot find the current once but the compiled files are already in tests/elfs
     (Taken from https://github.com/solana-labs/rbpf/blob/main/tests/elfs/elfs.sh)
     - /solana/sdk/bpf/dependencies/bpf-tools/llvm/bin/clang -Werror -target bpf -O2 -fno-builtin -fPIC -o test_simple_add_vm.o -c test_simple_add.c
     - /solana/sdk/bpf/dependencies/bpf-tools/llvm/bin/ld.lld -z
notext -shared --Bdynamic -entry entrypoint -o test_simple_add_vm.so test_simple_add_vm.o
4. Start vm with debugging support:
    - cargo run --example test_gdb --features debug
5. Start gdb
    - ./gdb/gdb test_simple_add_gdb.o (in tests/elfs)
    - (gdb) set disassemble-next-line on
    - (gdb) target remote :9001
6. Debugging
     - (gdb) stepi/step - for instruction or line stepping
     - (gdb) i r - print registers + sp + pc
     - (gdb) i locals - print all variables in current scope (if not optimized out)
     - Breakpoint on instruction address - (gdb) b *0x20
     - Breakpoint on line - (gdb) b <line_nr>
     - (gdb) i func - list all available functions
     - (gdb) b <func_name> - set breakpoint at function entry
     - (gdb) set $<register_nr> = < value >    - edit register value (the test always expects return 5 so changing regs will return an error at the very end)
  
To inspect an object file to see all instructions: bpf-objdump -d <file_name> or with -S to see debug info aligned.  
(Any objdump will do (eg llvm-objdump(-12)), but the bpf one is showing the correct opcode mnemonic)
  
    

![](misc/rbpf_256.png)

Rust (user-space) virtual machine for eBPF

[![Build Status](https://travis-ci.org/solana-labs/rbpf.svg?branch=main)](https://travis-ci.org/solana-labs/rbpf)
[![Crates.io](https://img.shields.io/crates/v/solana_rbpf.svg)](https://crates.io/crates/solana_rbpf)

## Description

This is a fork of [RBPF](https://github.com/qmonnet/rbpf) by Quentin Monnet.

This crate contains a virtual machine for eBPF program execution. BPF, as in
_Berkeley Packet Filter_, is an assembly-like language initially developed for
BSD systems, in order to filter packets in the kernel with tools such as
tcpdump so as to avoid useless copies to user-space. It was ported to Linux,
where it evolved into eBPF (_extended_ BPF), a faster version with more
features. While BPF programs are originally intended to run in the kernel, the
virtual machine of this crate enables running it in user-space applications;
it contains an interpreter, an x86_64 JIT-compiler for eBPF programs, as well as
an assembler, disassembler and verifier.

The crate is supposed to compile and run on Linux, MacOS X, and Windows,
although the JIT-compiler does not work with Windows at this time.

## Link to the crate

This crate is available from [crates.io](https://crates.io/crates/solana_rbpf),
so it should work out of the box by adding it as a dependency in your
`Cargo.toml` file:

```toml
[dependencies]
solana_rbpf = "0.2.8"
```

You can also use the development version from this GitHub repository. This
should be as simple as putting this inside your `Cargo.toml`:

```toml
[dependencies]
solana_rbpf = { git = "https://github.com/solana-labs/rbpf" }
```

Of course, if you prefer, you can clone it locally, possibly hack the crate,
and then indicate the path of your local version in `Cargo.toml`:

```toml
[dependencies]
solana_rbpf = { path = "path/to/solana_rbpf" }
```

Then indicate in your source code that you want to use the crate:

```rust,ignore
extern crate solana_rbpf;
```

## API

The API is pretty well documented inside the source code. You should also be
able to access [an online version of the documentation from
here](https://docs.rs/solana_rbpf/), automatically generated from the
[crates.io](https://crates.io/crates/solana_rbpf)
version (may not be up-to-date with master branch).
[Examples](examples), [unit tests](tests) and [performance benchmarks](benches)
should also prove helpful.

Here are the steps to follow to run an eBPF program with rbpf:

1. Create an executable, either from the bytecode or an ELF.
2. Create a syscall-registry, add some syscalls and put it in the executable.
3. If you want a JIT-compiled program, compile it.
4. Create a memory mapping, consisting of multiple memory regions.
5. Create the config and a virtual machine using all of the previous steps.
   You can also pass a readonly memory here which will be mapped as packet data
   in the eBPF programs register at index one.
6. If you registered syscall functions then bind their context objects.
7. Create an instruction meter.
8. Execute your program: Either run the interpreter or call the JIT-compiled
   function.

## License

Following the effort of the Rust language project itself in order to ease
integration with other projects, the rbpf crate is distributed under the terms
of both the MIT license and the Apache License (Version 2.0).

See [LICENSE-APACHE](LICENSE-APACHE) and [LICENSE-MIT](LICENSE-MIT) for details.
