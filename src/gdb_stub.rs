use byteorder::{LittleEndian, ReadBytesExt};
use gdbstub::{
    arch::{Arch, RegId, Registers},
    target::{
        ext::{
            base::{
                singlethread::{ResumeAction, SingleThreadOps, StopReason},
                BaseOps, GdbInterrupt, SingleRegisterAccess, SingleRegisterAccessOps,
            },
            breakpoints::{Breakpoints, BreakpointsOps, SwBreakpoint, SwBreakpointOps},
            section_offsets::{Offsets, SectionOffsets, SectionOffsetsOps},
        },
        Target, TargetError, TargetResult,
    },
    DisconnectReason, GdbStub, GdbStubError,
};
use std::debug_assert;
use std::io::Cursor;
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;


pub fn start_debug_server(
    port: u16,
) -> (mpsc::SyncSender<VmReply>, mpsc::Receiver<VmRequest>) {
    let conn = wait_for_gdb_connection(port).unwrap();
    let (mut target, tx, rx) = DebugServer::new();

    std::thread::spawn(move || {
        let mut debugger = GdbStub::new(conn);

        match debugger.run(&mut target) {
            Ok(disconnect_reason) => match disconnect_reason {
                DisconnectReason::Disconnect => println!("GDB client disconnected."),
                DisconnectReason::Kill => println!("GDB client sent a kill command!"),
                DisconnectReason::TargetExited(code) => {
                    println!("Target exited with code {}!", code)
                }
                DisconnectReason::TargetTerminated(sig) => {
                    println!("Target terminated with signal {}!", sig)
                }
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

pub struct DebugServer {
    req: mpsc::SyncSender<VmRequest>,
    reply: mpsc::Receiver<VmReply>,
}

impl DebugServer {
    fn new() -> (Self, mpsc::SyncSender<VmReply>, mpsc::Receiver<VmRequest>) {
        let (req_tx, req_rx) = mpsc::sync_channel::<VmRequest>(0);
        let (reply_tx, reply_rx) = mpsc::sync_channel::<VmReply>(0);
        (
            DebugServer {
                req: req_tx,
                reply: reply_rx,
            },
            reply_tx,
            req_rx,
        )
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[repr(C)]
pub struct BpfRegs {
    pub regs: [u64; 11],
    pub pc: u64,
}

impl Registers for BpfRegs {
    type ProgramCounter = u64;

    fn pc(&self) -> Self::ProgramCounter {
        self.pc as u64
    }

    fn gdb_serialize(&self, mut write_byte: impl FnMut(Option<u8>)) {
        macro_rules! write_bytes {
            ($bytes:expr) => {
                for b in $bytes {
                    write_byte(Some(*b))
                }
            };
        }

        for reg in self.regs.iter() {
            write_bytes!(&reg.to_le_bytes());
        }
        write_bytes!(&self.pc.to_le_bytes());
    }

    fn gdb_deserialize(&mut self, _bytes: &[u8]) -> Result<(), ()> {
        Ok(())
    }
}

#[derive(Debug)]
pub struct BpfRegId(u8);
impl RegId for BpfRegId {
    fn from_raw_id(id: usize) -> Option<(Self, usize)> {
        if id < 13 {
            Some((BpfRegId(id as u8), 64))
        } else {
            None
        }
    }
}

impl From<u8> for BpfRegId {
    fn from(val: u8) -> BpfRegId {
        BpfRegId(val)
    }
}

impl From<BpfRegId> for u8 {
    fn from(val: BpfRegId) -> u8 {
        val.0
    }
}

#[derive(Debug)]
pub enum BpfBreakpointKind {
    BpfBpKindBrkpt,
}

impl gdbstub::arch::BreakpointKind for BpfBreakpointKind {
    fn from_usize(kind: usize) -> Option<Self> {
        let kind = match kind {
            0 => BpfBreakpointKind::BpfBpKindBrkpt,
            _ => return None,
        };
        Some(kind)
    }
}

pub enum Bpf {}

impl Arch for Bpf {
    type Usize = u64;
    type Registers = BpfRegs;
    type RegId = BpfRegId;
    type BreakpointKind = BpfBreakpointKind;

    fn target_description_xml() -> Option<&'static str> {
        Some(r#"<target version="1.0"><architecture>bpf</architecture></target>"#)
    }
}

impl Target for DebugServer {
    type Arch = Bpf;
    type Error = &'static str;

    #[inline(always)]
    fn base_ops(&mut self) -> BaseOps<Self::Arch, Self::Error> {
        BaseOps::SingleThread(self)
    }

    #[inline(always)]
    fn breakpoints(&mut self) -> Option<BreakpointsOps<Self>> {
        Some(self)
    }

    #[inline(always)]
    fn section_offsets(&mut self) -> Option<SectionOffsetsOps<Self>> {
        Some(self)
    }
}

#[allow(dead_code)]
pub enum VmRequest {
    Continue,
    Step,
    ReadReg(u8),
    ReadRegs,
    WriteReg(u8, u64),
    WriteRegs(BpfRegs),
    ReadMem(u64, u64),
    WriteMem(u64, u64, Vec<u8>),
    SetBrkpt(u64),
    RemoveBrkpt(u64),
}

#[allow(dead_code)]
pub enum VmReply {
    DoneStep,
    Interrupt,
    Halted(u8),
    Terminated,
    Breakpoint,
    Err(&'static str),
    ReadRegs(BpfRegs),
    ReadReg(u64),
    WriteRegs,
    WriteReg,
    ReadMem(&'static [u8]),
    WriteMem,
    SetBrkpt,
    RemoveBrkpt,
}

impl SingleThreadOps for DebugServer {
    fn resume(
        &mut self,
        action: ResumeAction,
        _check_gdb_interrupt: GdbInterrupt<'_>,
    ) -> Result<StopReason<u64>, Self::Error> {
        match action {
            ResumeAction::Step => {
                self.req.send(VmRequest::Step).unwrap();
                match self.reply.recv().unwrap() {
                    VmReply::DoneStep => return Ok(StopReason::DoneStep),
                    VmReply::Halted(ret_val) => {
                        return Ok(StopReason::Exited(ret_val));
                    }
                    _ => return Err("unexpected  from VM"),
                }
            }
            ResumeAction::Continue => {
                self.req.send(VmRequest::Continue).unwrap();
                loop {
                    match self.reply.try_recv() {
                        Ok(VmReply::Halted(ret_val)) => {
                            return Ok(StopReason::Exited(ret_val));
                        }
                        Ok(VmReply::Breakpoint) => return Ok(StopReason::SwBreak),
                        Ok(_) => continue,
                        Err(mpsc::TryRecvError::Disconnected) => (),
                        Err(mpsc::TryRecvError::Empty) => (),
                    }
                }
            }
            _ => return Err("cannot resume with signal"),
        };
    }
    fn single_register_access(&mut self) -> Option<SingleRegisterAccessOps<(), Self>> {
        Some(self)
    }

    fn read_registers(&mut self, registers: &mut BpfRegs) -> TargetResult<(), Self> {
        self.req.send(VmRequest::ReadRegs).unwrap();
        match self.reply.recv().unwrap() {
            VmReply::ReadRegs(BpfRegs { regs, pc }) => {
                registers.regs = regs;
                registers.pc = pc;
                Ok(())
            }
            VmReply::Err(e) => Err(TargetError::Fatal(e)),
            _ => Err(TargetError::Fatal("unexpected reply from VM")),
        }
    }

    fn write_registers(&mut self, regs: &BpfRegs) -> TargetResult<(), Self> {
        self.req.send(VmRequest::WriteRegs(*regs)).unwrap();
        match self.reply.recv().unwrap() {
            VmReply::WriteRegs => Ok(()),
            VmReply::Err(e) => Err(TargetError::Fatal(e)),
            _ => Err(TargetError::Fatal("unexpected reply from VM")),
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

impl SingleRegisterAccess<()> for DebugServer {
    fn read_register(
        &mut self,
        _tid: (),
        reg_id: BpfRegId,
        dst: &mut [u8],
    ) -> TargetResult<(), Self> {
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

    fn write_register(&mut self, _tid: (), reg_id: BpfRegId, val: &[u8]) -> TargetResult<(), Self> {
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
}

impl Breakpoints for DebugServer {
    fn sw_breakpoint(&mut self) -> Option<SwBreakpointOps<Self>> {
        Some(self)
    }
}

impl SwBreakpoint for DebugServer {
    fn add_sw_breakpoint(
        &mut self,
        addr: u64,
        _kind: BpfBreakpointKind,
    ) -> TargetResult<bool, Self> {
        self.req.send(VmRequest::SetBrkpt(addr)).unwrap();
        match self.reply.recv().unwrap() {
            VmReply::SetBrkpt => Ok(true),
            VmReply::Err(e) => Err(TargetError::Fatal(e)),
            _ => Err(TargetError::Fatal("unexpected reply from VM")),
        }
    }

    fn remove_sw_breakpoint(
        &mut self,
        addr: u64,
        _kind: BpfBreakpointKind,
    ) -> TargetResult<bool, Self> {
        self.req.send(VmRequest::RemoveBrkpt(addr)).unwrap();
        match self.reply.recv().unwrap() {
            VmReply::RemoveBrkpt => Ok(true),
            VmReply::Err(e) => Err(TargetError::Fatal(e)),
            _ => Err(TargetError::Fatal("unexpected reply from VM")),
        }
    }
}

impl SectionOffsets for DebugServer {
    fn get_section_offsets(&mut self) -> Result<Offsets<u64>, Self::Error> {
        Ok(Offsets::Sections {
            text: 0,
            data: 0,
            bss: None,
        })
    }
}
