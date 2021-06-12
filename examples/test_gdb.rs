// Copyright 2020 Solana Maintainers <maintainers@solana.com>
//
// Licensed under the Apache License, Version 2.0 <http://www.apache.org/licenses/LICENSE-2.0> or
// the MIT license <http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

extern crate byteorder;
extern crate libc;
extern crate solana_rbpf;
extern crate test_utils;
extern crate thiserror;

#[cfg(not(windows))]
use solana_rbpf::{
    user_error::UserError,
    vm::{Config, EbpfVm, Executable, SyscallRegistry},
};
use std::{fs::File, io::Read};
use test_utils::{Result, TestInstructionMeter};

macro_rules! test_interpreter_and_jit {
    (register, $executable:expr, $syscall_registry:expr, $location:expr => $syscall_function:expr; $syscall_context_object:expr) => {
        $syscall_registry.register_syscall_by_name::<UserError, _>($location, $syscall_function).unwrap();
    };
    (bind, $vm:expr, $location:expr => $syscall_function:expr; $syscall_context_object:expr) => {
        $vm.bind_syscall_context_object(Box::new($syscall_context_object), None).unwrap();
    };
    ($executable:expr, $mem:tt, ($($location:expr => $syscall_function:expr; $syscall_context_object:expr),* $(,)?), $check:block, $expected_instruction_count:expr) => {
        let check_closure = $check;
        let mut syscall_registry = SyscallRegistry::default();
        $(test_interpreter_and_jit!(register, $executable, syscall_registry, $location => $syscall_function; $syscall_context_object);)*
        $executable.set_syscall_registry(syscall_registry);
        let (instruction_count_interpreter, _tracer_interpreter) = {
            let mut mem = $mem;
            let mut vm = EbpfVm::new($executable.as_ref(), &mut mem, &[]).unwrap();
            $(test_interpreter_and_jit!(bind, vm, $location => $syscall_function; $syscall_context_object);)*
            let result = vm.execute_program_interpreted(&mut TestInstructionMeter { remaining: $expected_instruction_count });
            assert!(check_closure(&vm, result));
            (vm.get_total_instruction_count(), vm.get_tracer().clone())
        };
        #[cfg(not(windows))]
        {
            let check_closure = $check;
            let compilation_result = $executable.jit_compile();
            let mut mem = $mem;
            let mut vm = EbpfVm::new($executable.as_ref(), &mut mem, &[]).unwrap();
            match compilation_result {
                Err(err) => assert!(check_closure(&vm, Err(err))),
                Ok(()) => {
                    $(test_interpreter_and_jit!(bind, vm, $location => $syscall_function; $syscall_context_object);)*
                    let result = vm.execute_program_jit(&mut TestInstructionMeter { remaining: $expected_instruction_count });
                    let tracer_jit = vm.get_tracer();
                    if !check_closure(&vm, result) || !solana_rbpf::vm::Tracer::compare(&_tracer_interpreter, tracer_jit) {
                        let analysis = solana_rbpf::static_analysis::Analysis::from_executable($executable.as_ref());
                        let stdout = std::io::stdout();
                        _tracer_interpreter.write(&mut stdout.lock(), &analysis).unwrap();
                        tracer_jit.write(&mut stdout.lock(), &analysis).unwrap();
                        panic!();
                    }
                    if $executable.get_config().enable_instruction_meter {
                        let instruction_count_jit = vm.get_total_instruction_count();
                        assert_eq!(instruction_count_interpreter, instruction_count_jit);
                    }
                },
            }
        }
        if $executable.get_config().enable_instruction_meter {
            assert_eq!(instruction_count_interpreter, $expected_instruction_count);
        }
    };
}

macro_rules! test_interpreter_and_jit_elf {
    ($source:tt, $mem:tt, ($($location:expr => $syscall_function:expr; $syscall_context_object:expr),* $(,)?), $check:block, $expected_instruction_count:expr) => {
        let mut file = File::open($source).unwrap();
        let mut elf = Vec::new();
        file.read_to_end(&mut elf).unwrap();
        #[allow(unused_mut)]
        {
            let config = Config {
                enable_instruction_tracing: true,
                ..Config::default()
            };
            let mut executable = <dyn Executable::<UserError, TestInstructionMeter>>::from_elf(&elf, None, config).unwrap();
            test_interpreter_and_jit!(executable, $mem, ($($location => $syscall_function; $syscall_context_object),*), $check, $expected_instruction_count);
        }
    };
}

fn main() {
    test_interpreter_and_jit_elf!(
        //        "tests/elfs/relative_call.so",
        "tests/elfs/test_simple_add_vm.so",
        [],
        (
                //b"log" => BpfSyscallString::call; BpfSyscallString {},
            ),
        { |_vm, res: Result| { res.unwrap() == 5 } },
        12
    );
}
