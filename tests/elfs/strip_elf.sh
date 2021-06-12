#!/bin/bash

llvm-objcopy-12 --remove-section .rel.debug_line test_simple_add_gdb.o
llvm-objcopy-12 --remove-section .rel.debug_info  test_simple_add_gdb.o
llvm-objcopy-12 --remove-section .rel.debug_info  test_simple_add_gdb.o
llvm-objcopy-12 --remove-section .eh_frame test_simple_add_gdb.o
llvm-objcopy-12 --remove-section .rel.BTF.ext test_simple_add_gdb.o
