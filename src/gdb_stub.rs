use byteorder::{LittleEndian, ReadBytesExt};
use gdbstub::{
    arch::{Arch, RegId, Registers},
    target::{
        ext::{
            base::{
                singlethread::{ResumeAction, SingleThreadOps, StopReason},
                BaseOps,
            },
            breakpoints::{SwBreakpoint, SwBreakpointOps},
            section_offsets::{Offsets, SectionOffsets, SectionOffsetsOps},
        },
        Target, TargetError, TargetResult,
    },
    DisconnectReason, GdbStub, GdbStubError,
};
use std::collections::HashSet;
use std::debug_assert;
use std::io::Cursor;
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;

const BRPKT_MAP_THRESH: usize = 30;

const NUM_REGS: usize = 11;
const NUM_REGS_WITH_PC: usize = 12;
const REG_SIZE: usize = 8;
const REG_NUM_BYTES: usize = NUM_REGS * REG_SIZE;
const REG_WITH_PC_NUM_BYTES: usize = NUM_REGS * REG_SIZE;

// TODO make this not use unwrap
// TODO add support for Unix Domain Sockets
pub fn start_debug_server(
    port: u16,
    init_regs: &[u64; 11],
    init_pc: u64,
) -> (mpsc::SyncSender<VmReply>, mpsc::Receiver<VmRequest>) {
    let conn = wait_for_gdb_connection(port).unwrap();
    let (mut target, tx, rx) = DebugServer::new(init_regs, init_pc);

    std::thread::spawn(move || {
        let mut debugger = GdbStub::new(conn);

        println!("Thread started");
        match debugger.run(&mut target) {
            Ok(disconnect_reason) => match disconnect_reason {
                DisconnectReason::Disconnect => println!("GDB client disconnected."),
                DisconnectReason::TargetHalted => println!("Target halted!"),
                DisconnectReason::Kill => println!("GDB client sent a kill command!"),
            },
            // Handle any target-specific errors
            Err(GdbStubError::TargetError(e)) => {
                println!("Target raised a fatal error: {:?}", e);
                // e.g: re-enter the debugging session after "freezing" a system to
                // conduct some post-mortem debugging
                debugger.run(&mut target).unwrap();
            }
            Err(e) => {
                eprint!("Could not run Target {:?}\n", e);
            }
        }
    });
    std::thread::sleep(std::time::Duration::from_millis(10000));
    (tx, rx)
}

fn wait_for_gdb_connection(port: u16) -> std::io::Result<TcpStream> {
    let sockaddr = format!("localhost:{}", port);
    eprintln!("Waiting for a GDB connection on {:?}...", sockaddr);
    let sock = TcpListener::bind(sockaddr)?;
    let (stream, addr) = sock.accept()?;

    // Blocks until a GDB client connects via TCP.
    // i.e: Running `target remote localhost:<port>` from the GDB prompt.

    eprintln!("Debugger connected from {}", addr);
    Ok(stream)
}

pub enum BreakpointTable {
    Few(Vec<u64>),
    Many(HashSet<u64>),
}

impl BreakpointTable {
    pub fn new() -> Self {
        BreakpointTable::Few(Vec::new())
    }

    pub fn check_breakpoint(&self, addr: u64) -> bool {
        match &*self {
            BreakpointTable::Few(addrs) => {
                for brkpt_addr in addrs.iter() {
                    if *brkpt_addr == addr {
                        return true;
                    }
                }
                return false;
            }
            BreakpointTable::Many(addrs) => addrs.contains(&addr),
        }
    }

    pub fn set_breakpoint(&mut self, addr: u64) {
        match *self {
            BreakpointTable::Few(ref mut addrs) => {
                if addrs.len() > BRPKT_MAP_THRESH {
                    let mut set = HashSet::<u64>::with_capacity(addrs.len() + 1);
                    set.insert(addr);
                    for addr in addrs.iter() {
                        set.insert(*addr);
                    }
                    *self = BreakpointTable::Many(set);
                } else {
                    addrs.push(addr);
                }
            }
            BreakpointTable::Many(ref mut addrs) => {
                addrs.insert(addr);
            }
        }
    }

    pub fn remove_breakpoint(&mut self, addr: u64) {
        match *self {
            BreakpointTable::Few(ref mut addrs) => {
                if let Some(i) =
                    addrs
                        .iter()
                        .enumerate()
                        .find_map(|(i, address)| if *address == addr { Some(i) } else { None })
                {
                    addrs.remove(i);
                }
            }
            BreakpointTable::Many(ref mut addrs) => {
                addrs.remove(&addr);
            }
        }
    }
}

pub struct DebugServer {
    req: mpsc::SyncSender<VmRequest>,
    reply: mpsc::Receiver<VmReply>,
    regs: BPFRegs,
}

impl DebugServer {
    fn new(
        regs: &[u64; 11],
        pc: u64,
    ) -> (Self, mpsc::SyncSender<VmReply>, mpsc::Receiver<VmRequest>) {
        let (req_tx, req_rx) = mpsc::sync_channel::<VmRequest>(0);
        let (reply_tx, reply_rx) = mpsc::sync_channel::<VmReply>(0);
        (
            DebugServer {
                req: req_tx,
                reply: reply_rx,
                regs: BPFRegs {
                    regs: *regs,
                    pc: pc,
                },
            },
            reply_tx,
            req_rx,
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
#[repr(C)]
pub struct BPFRegs {
    regs: [u64; 11],
    pc: u64,
}

// TODO use something safer than transmute_copy
impl Registers for BPFRegs {
    fn gdb_serialize(&self, mut write_byte: impl FnMut(Option<u8>)) {
        let bytes: [u8; REG_WITH_PC_NUM_BYTES] = unsafe { std::mem::transmute_copy(self) };
        bytes.iter().for_each(|b| write_byte(Some(*b)));
    }

    fn gdb_deserialize(&mut self, bytes: &[u8]) -> Result<(), ()> {
        let mut rdr = Cursor::new(bytes);
        let mut acc = BPFRegs::default();
        for i in 0..NUM_REGS {
            if let Ok(u) = rdr.read_u64::<LittleEndian>() {
                acc.regs[i] = u;
            } else {
                return Err(());
            }
        }
        if let Ok(u) = rdr.read_u64::<LittleEndian>() {
            acc.pc = u;
            *self = acc;
            Ok(())
        } else {
            Err(())
        }
    }
}

#[derive(Debug)]
pub struct BPFRegId(u8);
impl RegId for BPFRegId {
    fn from_raw_id(id: usize) -> Option<(Self, usize)> {
        if id < 13 {
            Some((BPFRegId(id as u8), 64))
        } else {
            None
        }
    }
}

impl From<u8> for BPFRegId {
    fn from(val: u8) -> BPFRegId {
        BPFRegId(val)
    }
}

impl From<BPFRegId> for u8 {
    fn from(val: BPFRegId) -> u8 {
        val.0
    }
}

pub struct BPFArch;

impl Arch for BPFArch {
    type Usize = u64;
    type Registers = BPFRegs;
    type RegId = BPFRegId;
}

impl Target for DebugServer {
    type Arch = BPFArch;
    type Error = &'static str;

    fn base_ops(&mut self) -> BaseOps<Self::Arch, Self::Error> {
        BaseOps::SingleThread(self)
    }

    fn sw_breakpoint(&mut self) -> Option<SwBreakpointOps<Self>> {
        Some(self)
    }

    fn section_offsets(&mut self) -> Option<SectionOffsetsOps<Self>> {
        Some(self)
    }
}

pub enum VmRequest {
    Resume,
    Interrupt,
    Step,
    ReadReg(u8),
    ReadRegs,
    WriteReg(u8, u64),
    WriteRegs([u64; 12]),
    ReadMem(u64, u64),
    WriteMem(u64, u64, Vec<u8>),
    SetBrkpt(u64),
    RemoveBrkpt(u64),
    Offsets,
    Detatch,
}

pub enum VmReply {
    DoneStep,
    Interrupt,
    Halted,
    Breakpoint,
    Err(&'static str),
    ReadRegs([u64; 12]),
    ReadReg(u64),
    WriteRegs,
    WriteReg,
    ReadMem(Vec<u8>),
    WriteMem,
    SetBrkpt,
    RemoveBrkpt,
    Offsets(Offsets<u64>),
}

// TODO make this not use unwrap
impl SingleThreadOps for DebugServer {
    fn resume(
        &mut self,
        action: ResumeAction,
        check_gdb_interrupt: &mut dyn FnMut() -> bool,
    ) -> Result<StopReason<u64>, Self::Error> {
        println!("RESUME");
        match action {
            ResumeAction::Step => {
                self.req.send(VmRequest::Step).unwrap();
                match self.reply.recv().unwrap() {
                    VmReply::DoneStep => Ok(StopReason::DoneStep),
                    _ => Err("unexpected  from VM"),
                }
            }
            ResumeAction::Continue => {
                println!("CONTINUE");
                self.req.send(VmRequest::Resume).unwrap();
                // TODO find a better way to deal with check_gdb_interrupt
                /*while !check_gdb_interrupt() {
                    if let Ok(event) = self.reply.try_recv() {
                        return match event {
                            VmReply::Breakpoint => Ok(StopReason::SwBreak),
                            VmReply::Halted => Ok(StopReason::Halted),
                            VmReply::Err(e) => Err(e),
                            _ => Err("unexpected reply from VM"),
                        };
                    }
                }*/
                //self.req.send(VmRequest::Interrupt).unwrap();
                println!("CONTINUE END");
                /*match self.reply.recv().unwrap() {
                    VmReply::Interrupt => Ok(StopReason::GdbInterrupt),
                    VmReply::Err(e) => Err(e),
                    _ => Err("unexpected reply from VM"),
                }*/
                Ok(StopReason::GdbInterrupt)
            }
        }
    }

    fn read_registers(&mut self, regs: &mut BPFRegs) -> TargetResult<(), Self> {
        println!("READ REGISTERSSS");
        for i in 0..11 {
            regs.regs[i] = self.regs.regs[i];
        }
        regs.pc = self.regs.pc;
        /*self.req.send(VmRequest::ReadRegs).unwrap();
        match self.reply.recv().unwrap() {
            VmReply::ReadRegs(regfile) => {
                *regs = unsafe { std::mem::transmute_copy(&regfile) };
                Ok(())
            }
            VmReply::Err(e) => Err(TargetError::Fatal(e)),
            _ => Err(TargetError::Fatal("unexpected reply from VM")),
        }*/
        Ok(())
    }

    fn write_registers(&mut self, regs: &BPFRegs) -> TargetResult<(), Self> {
        let regfile: [u64; 12] = unsafe { std::mem::transmute_copy(regs) };
        self.req.send(VmRequest::WriteRegs(regfile)).unwrap();
        match self.reply.recv().unwrap() {
            VmReply::WriteRegs => Ok(()),
            VmReply::Err(e) => Err(TargetError::Fatal(e)),
            _ => Err(TargetError::Fatal("unexpected reply from VM")),
        }
    }

    fn read_register(&mut self, reg_id: BPFRegId, dst: &mut [u8]) -> TargetResult<(), Self> {
        self.req.send(VmRequest::ReadReg(reg_id.into())).unwrap();
        match self.reply.recv().unwrap() {
            VmReply::ReadReg(val) => {
                dst.copy_from_slice(&val.to_le_bytes());
                Ok(())
            }
            VmReply::Err(e) => Err(TargetError::Fatal(e)),
            _ => Err(TargetError::Fatal("unexpected reply from VM")),
        }
    }

    fn write_register(&mut self, reg_id: BPFRegId, val: &[u8]) -> TargetResult<(), Self> {
        let mut rdr = Cursor::new(val);
        match rdr.read_u64::<LittleEndian>() {
            Ok(reg) => {
                self.req
                    .send(VmRequest::WriteReg(reg_id.into(), reg))
                    .unwrap();
                match self.reply.recv().unwrap() {
                    VmReply::WriteReg => Ok(()),
                    VmReply::Err(e) => Err(TargetError::Fatal(e)),
                    _ => Err(TargetError::Fatal("unexpected reply from VM")),
                }
            }
            _ => Err(TargetError::Fatal("invalid number of bytes")),
        }
    }

    fn read_addrs(&mut self, start_addr: u64, dst: &mut [u8]) -> TargetResult<(), Self> {
        self.req
            .send(VmRequest::ReadMem(start_addr, dst.len() as u64))
            .unwrap();
        match self.reply.recv().unwrap() {
            VmReply::ReadMem(bytes) => {
                debug_assert!(
                    bytes.len() == dst.len(),
                    "vm returned wrong number of bytes!"
                );
                dst.copy_from_slice(&bytes[..]);
                Ok(())
            }
            VmReply::Err(e) => Err(TargetError::Fatal(e)),
            _ => Err(TargetError::Fatal("unexpected reply from VM")),
        }
    }

    fn write_addrs(&mut self, start_addr: u64, data: &[u8]) -> TargetResult<(), Self> {
        self.req
            .send(VmRequest::WriteMem(
                start_addr,
                data.len() as u64,
                data.to_vec(),
            ))
            .unwrap();
        match self.reply.recv().unwrap() {
            VmReply::WriteMem => Ok(()),
            VmReply::Err(e) => Err(TargetError::Fatal(e)),
            _ => Err(TargetError::Fatal("unexpected reply from VM")),
        }
    }
}

// TODO make this not use unwrap
impl SwBreakpoint for DebugServer {
    fn add_sw_breakpoint(&mut self, addr: u64) -> TargetResult<bool, Self> {
        self.req.send(VmRequest::SetBrkpt(addr)).unwrap();
        match self.reply.recv().unwrap() {
            VmReply::SetBrkpt => Ok(true),
            VmReply::Err(e) => Err(TargetError::Fatal(e)),
            _ => Err(TargetError::Fatal("unexpected reply from VM")),
        }
    }

    fn remove_sw_breakpoint(&mut self, addr: u64) -> TargetResult<bool, Self> {
        self.req.send(VmRequest::RemoveBrkpt(addr)).unwrap();
        match self.reply.recv().unwrap() {
            VmReply::RemoveBrkpt => Ok(true),
            VmReply::Err(e) => Err(TargetError::Fatal(e)),
            _ => Err(TargetError::Fatal("unexpected reply from VM")),
        }
    }
}

// TODO make this not use unwrap
impl SectionOffsets for DebugServer {
    fn get_section_offsets(&mut self) -> Result<Offsets<u64>, Self::Error> {
        println!("OFFSETS");
        Ok(Offsets::Sections {
            text: 0,
            data: 0,
            bss: None,
        }) /*
           self.req.send(VmRequest::Offsets).unwrap();
           match self.reply.recv().unwrap() {
               VmReply::Offsets(offsets) => Ok(offsets),
               VmReply::Err(e) => Err(e),
               _ => Err("unexpected reply from VM"),
           }*/
    }
}
