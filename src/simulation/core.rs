//! Simulation runtime (aka `Core`) and tools to build a core

use std::iter::FromIterator;
use std::collections::{VecDeque, HashMap};

use redcode::*;

pub type CoreResult<T> = Result<T, ()>;

/// Events that can happen during a running simulation
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum CoreEvent
{
    /// All processes terminated successfully
    Finished,

    /// Game ended in a tie
    Tied,

    /// Process split inner contains address of new pc
    Split,

    /// A process terminated
    Terminated(Pid),

    /// A process jumped address
    Jumped,

    /// Skipped happens in all `Skip if ...` instructions
    Skipped,

    /// Nothing happened
    Stepped,
}

/// Core wars runtime
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Core
{
    /// Core memory
    pub(super) memory:        Vec<Instruction>,

    /// Current process id being run
    pub(super) current_pid:   Pid,

    /// Current program counter
    pub(super) pc:            Address,

    /// Instruction register
    pub(super) ir:            Instruction,

    /// Current process queue
    pub(super) current_queue: VecDeque<Address>,

    /// Current numbered cycle core is executing
    pub(super) current_cycle: usize,

    /// Program counter for each process currently loaded into memory
    pub(super) process_queue: VecDeque<(Pid, VecDeque<Address>)>,

    /// Private storage space for warriors
    pub(super) pspace:        HashMap<Pin, Vec<Instruction>>,

    /// Has the core finished executing
    pub(super) finished:      bool,

    // Load constraints
    pub(super) max_length:    usize,

    /// Size of P-space
    pub(super) pspace_size:   usize,

    // Runtime constraints
    /// Core version
    pub(super) version:       usize,

    /// Maximum of processes that can be on the process queue at any time
    pub(super) max_processes: usize,

    /// Maximum number of cycles that can pass before a tie is declared
    pub(super) max_cycles:    usize,
}

impl Core
{
    /// Step forward one cycle
    ///
    /// # Examples
    /// ```
    /// use libcw::simulation::{CoreBuilder, CoreEvent};
    /// use libcw::redcode::*;
    ///
    /// let imp = vec![
    ///     Instruction {
    ///         op: OpField {
    ///             code: OpCode::Mov,
    ///             mode: OpMode::I
    ///         },
    ///         a: Field {
    ///             offset: 0,
    ///             mode: AddressingMode::Direct,
    ///         },
    ///         b: Field {
    ///             offset: 1,
    ///             mode: AddressingMode::Direct
    ///         }
    ///     },
    /// ];
    ///
    /// let mut core = CoreBuilder::new()
    ///     .build_and_load(vec![
    ///         (0, None, imp.clone()),
    ///         (4000, None, imp.clone())
    ///     ])
    ///     .unwrap();
    ///
    /// // Stepping the core forward will step the pc forward 1
    /// let event = core.step();
    /// assert_eq!(Ok(CoreEvent::Stepped), event);
    ///
    /// ```
    pub fn step(&mut self) -> CoreResult<CoreEvent>
    {
        if self.finished() { // can't step after the core is halted
            return Err(());
        }

        if self.cycle() >= self.max_cycles() {
            self.finished = true;
            return Ok(CoreEvent::Tied)
        }

        // Fetch instruction
        self.ir = self.fetch(self.pc);
        let (a_mode, b_mode) = (self.ir.a.mode, self.ir.b.mode);

        // PostIncrement phase
        let predecrement = a_mode == AddressingMode::AIndirectPreDecrement ||
            a_mode == AddressingMode::BIndirectPreDecrement ||
            b_mode == AddressingMode::AIndirectPreDecrement ||
            b_mode == AddressingMode::BIndirectPreDecrement;

        // Preincrement phase
        if predecrement {
            // fetch direct target
            let a_addr = self.calc_addr_offset(self.pc, self.ir.a.offset);
            let b_addr = self.calc_addr_offset(self.pc, self.ir.b.offset);
            let mut a = self.fetch(a_addr);
            let mut b = self.fetch(b_addr);

            // FIXME: combine these into a single match statement
            match a_mode {
                AddressingMode::AIndirectPreDecrement => a.a.offset -= 1,
                AddressingMode::BIndirectPreDecrement => a.b.offset -= 1,
                _ => { /* Do nothing */ }
            };

            match b_mode {
                AddressingMode::AIndirectPreDecrement => b.a.offset -= 1,
                AddressingMode::BIndirectPreDecrement => b.b.offset -= 1,
                _ => { /* Do nothing */ }
            };
            self.store(a_addr, a);
            self.store(b_addr, b);
        }

        // Execute instruction(updating the program counter and requeing it
        // are handled in this phase)
        let exec_event = self.execute();

        // PostIncrement phase
        let postincrement = a_mode == AddressingMode::AIndirectPostIncrement ||
            a_mode == AddressingMode::BIndirectPostIncrement ||
            b_mode == AddressingMode::AIndirectPostIncrement ||
            b_mode == AddressingMode::BIndirectPostIncrement;

        if postincrement {
            // fetch direct target
            let a_addr = self.calc_addr_offset(self.pc, self.ir.a.offset);
            let b_addr = self.calc_addr_offset(self.pc, self.ir.b.offset);
            let mut a = self.fetch(a_addr);
            let mut b = self.fetch(b_addr);

            // FIXME: combine these into a single match statement
            match a_mode {
                AddressingMode::AIndirectPostIncrement => a.a.offset += 1,
                AddressingMode::BIndirectPostIncrement => a.b.offset += 1,
                _ => { /* Do nothing */ }
            };

            match b_mode {
                AddressingMode::AIndirectPostIncrement => b.a.offset += 1,
                AddressingMode::BIndirectPostIncrement => b.b.offset += 1,
                _ => { /* Do nothing */ }
            };
            // store result
            self.store(a_addr, a);
            self.store(b_addr, b);
        }

        // check if there are any more process queues running on the core
        if !self.current_queue.is_empty() {
            let q_entry = (self.pid(), self.current_queue.clone());
            self.process_queue.push_front(q_entry);
        }

        // check if there is only one PID remaining on the process queue
        if self.process_queue.len() <= 1 {
            self.finished = true;
            return Ok(CoreEvent::Finished);
        }

        // Fetch new queue
        let (pid, q)       = self.process_queue.pop_back().unwrap();
        self.current_queue = q;

        // Update pid and program counter
        self.pc          = self.current_queue.pop_back().unwrap();
        self.current_pid = pid;

        self.current_cycle += 1;
        Ok(exec_event)
    }

    /// Has the core finished its execution. This can mean either a tie has
    /// occurred or a warrior has emerged victoriors
    ///
    /// # Examples
    /// ```
    /// use libcw::simulation::CoreBuilder;
    ///
    /// // load no programs, meaning that the core is already finished
    /// let mut core = CoreBuilder::new().build_and_load(vec![]).unwrap();
    /// core.halt();
    ///
    /// assert_eq!(true, core.finished());
    /// 
    /// // stepping the core after it has finished results in an error
    /// assert_eq!(Err(()), core.step());
    ///
    /// ```
    pub fn finished(&mut self) -> bool
    {
        self.finished
    }

    /// Halt the Core
    pub fn halt(&mut self) -> &mut Self
    {
        self.finished = true;
        self
    }

    /// Reset the core
    ///
    /// # Arguments
    /// * `programs`: programs packed with pins and base load address
    pub fn reset(&mut self, programs: Vec<(Address, Option<Pin>, Program)>)
        -> Result<(), ()>
    {
        self.validate(&programs)?;

        // reset all assets
        self.process_queue.clear();
        self.current_cycle = 0;

        // reset memory
        for e in self.memory.iter_mut() {
            *e = Instruction::default()
        }

        // reload programs
        for &(base, _, ref prog) in programs.iter() {
            // copy into memory
            for i in 0..prog.len() {
                self.memory[i + base as usize] = prog[i];
            }
        }

        // Reload process queue
        for (i, &(base, _, _)) in programs.iter().enumerate() {
            let pid = i as Pid; 
            let mut q = VecDeque::new();
            q.push_front(base);

            let q_entry = (pid, q);
            self.process_queue.push_front(q_entry);
        }

        // Prepare current queue
        let (pid, curr_q) = self.process_queue.pop_back()
            .unwrap_or((0, VecDeque::new()));
        self.current_pid = pid;
        self.current_queue = curr_q;
        self.pc = self.current_queue.pop_back().unwrap_or(0);
        self.finished = false;

        Ok(())
    }

    pub fn reset_hard(&mut self, programs: Vec<(Address, Option<Pin>, Program)>)
        -> Result<(), ()>
    {
        // Reset pspace
        self.pspace.clear();

        for (i, &(_, maybe_pin, _)) in programs.iter().enumerate() {
            let pin = maybe_pin.unwrap_or(i as Pin);

            self.pspace.insert(
                pin,
                vec![Instruction::default(); self.pspace_size]
                );
        }

        self.reset(programs)
    }

    /// Validate that programs do not violate runtime constraints
    ///
    /// # Arguments
    /// * `programs`:
    fn validate(&self, programs: &Vec<(Address, Option<Pin>, Program)>)
        -> Result<(), ()>
    {
        let all_valid_length = programs.iter()
            .any(|&(_, _, ref prog)| prog.len() <= self.max_length);

        if !all_valid_length {
            return Err(());
        }

        // TODO: actually check spacing
        let valid_spacing = true;
        if !valid_spacing {
            return Err(());
        }

        Ok(())
    }

    /// Get `Pid` currently executing on the core
    /// # Example
    /// ```
    /// use libcw::simulation::CoreBuilder;
    /// use libcw::redcode::*;
    ///
    /// let imp = vec![
    ///     Instruction {
    ///         op: OpField {
    ///             code: OpCode::Mov,
    ///             mode: OpMode::I
    ///         },
    ///         a: Field {
    ///             offset: 0,
    ///             mode: AddressingMode::Direct,
    ///         },
    ///         b: Field {
    ///             offset: 1,
    ///             mode: AddressingMode::Direct
    ///         }
    ///     },
    /// ];
    ///
    /// let mut core = CoreBuilder::new().build_and_load(vec![
    ///     (0, None, imp.clone()),
    ///     (4000, None, imp.clone())
    ///     ])
    ///     .unwrap();
    ///
    /// // inital program counter is 0, first program was loaded in at address 0
    /// assert_eq!(0, core.pc());
    /// let _ = core.step();
    /// // Goes to next processes's program counter 
    /// assert_eq!(4000, core.pc());
    /// ```
    ///
    pub fn pc(&self) -> Address
    {
        self.pc
    }

    /// Get the program counters for all processes
    pub fn pcs(&self) -> Vec<(Pid, Address)>
    {
        unimplemented!();
    }

    /// Current cycle core is executing
    ///
    /// # Examples
    /// ```
    /// use libcw::simulation::{CoreBuilder, CoreEvent};
    /// use libcw::redcode::*;
    ///
    /// let imp = vec![
    ///     Instruction {
    ///         op: OpField {
    ///             code: OpCode::Mov,
    ///             mode: OpMode::I
    ///         },
    ///         a: Field {
    ///             offset: 0,
    ///             mode: AddressingMode::Direct,
    ///         },
    ///         b: Field {
    ///             offset: 1,
    ///             mode: AddressingMode::Direct
    ///         }
    ///     },
    /// ];
    ///
    /// let mut core = CoreBuilder::new().build_and_load(vec![
    ///     (0, None, imp.clone()),
    ///     (4000, None, imp.clone())
    ///     ])
    ///     .unwrap();
    ///
    /// // initial cycle is 0
    /// assert_eq!(0, core.cycle());
    /// let _ = core.step();
    /// // We're on the next cycle now
    /// assert_eq!(1, core.cycle());
    /// ```
    pub fn cycle(&self) -> usize
    {
        self.current_cycle
    }

    /// Get the current `Pid` executing
    ///
    /// # Example
    /// ```
    /// use libcw::simulation::{CoreBuilder, CoreEvent};
    /// use libcw::redcode::*;
    ///
    /// let imp = vec![
    ///     Instruction {
    ///         op: OpField {
    ///             code: OpCode::Mov,
    ///             mode: OpMode::I
    ///         },
    ///         a: Field {
    ///             offset: 0,
    ///             mode: AddressingMode::Direct,
    ///         },
    ///         b: Field {
    ///             offset: 1,
    ///             mode: AddressingMode::Direct
    ///         }
    ///     },
    /// ];
    ///
    /// let mut core = CoreBuilder::new().build_and_load(vec![
    ///     (0, None, imp.clone()),
    ///     (4000, None, imp.clone())
    ///     ])
    ///     .unwrap();
    ///
    /// // initial pid executing is 0
    /// assert_eq!(0, core.pid());
    /// let _ = core.step();
    /// // Next pid is 0
    /// assert_eq!(1, core.pid());
    /// ```
    pub fn pid(&self) -> Pid
    {
        self.current_pid
    }

    /// Get all `Pid`s that are currently active in the order they will be 
    /// executing
    ///
    /// # Example
    /// ```
    /// use std::collections::HashSet;
    /// use libcw::simulation::CoreBuilder;
    /// use libcw::redcode::*;
    ///
    /// let imp = vec![
    ///     Instruction {
    ///         op: OpField {
    ///             code: OpCode::Mov,
    ///             mode: OpMode::I
    ///         },
    ///         a: Field {
    ///             offset: 0,
    ///             mode: AddressingMode::Direct,
    ///         },
    ///         b: Field {
    ///             offset: 1,
    ///             mode: AddressingMode::Direct
    ///         }
    ///     },
    /// ];
    ///
    /// let mut core = CoreBuilder::new().build_and_load(vec![
    ///     (0, None, imp.clone()),
    ///     (4000, None, imp.clone())
    ///     ])
    ///     .unwrap();
    ///
    /// // Two programs were loaded, so two pids are running, 0 is next to exec
    /// assert_eq!(vec![0, 1], core.pids());
    /// let _ = core.step();
    /// // There are still 2 pids executing on the core, but 1 is the current
    /// assert_eq!(vec![1, 0], core.pids());
    /// ```
    pub fn pids(&self) -> Vec<Pid>
    {
        let mut pids = vec![self.pid()];
        pids.extend(self.process_queue.iter().map(|&(pid, _)| pid));
        pids
    }

    /// Size of memory
    ///
    /// # Example
    /// ```
    /// use libcw::simulation::CoreBuilder;
    ///
    /// let core = CoreBuilder::new()
    ///     .core_size(100)
    ///     .build_and_load(vec![])
    ///     .unwrap();
    ///
    /// assert_eq!(core.size(), 100);
    /// ```
    pub fn size(&self) -> usize
    {
        self.memory.len()
    }

    /// Version of core multiplied by `100`
    ///
    /// # Example
    /// ```
    /// use libcw::simulation::CoreBuilder;
    ///
    /// let core = CoreBuilder::new()
    ///     .version(800)
    ///     .build_and_load(vec![])
    ///     .unwrap();
    ///
    /// assert_eq!(core.version(), 800);
    /// ```
    pub fn version(&self) -> usize
    {
        self.version
    }

    /// Maximum number of processes that can be in the core queue
    ///
    /// # Example
    /// ```
    /// use libcw::simulation::CoreBuilder;
    ///
    /// let core = CoreBuilder::new()
    ///     .max_processes(800)
    ///     .build_and_load(vec![])
    ///     .unwrap();
    ///
    /// assert_eq!(core.max_processes(), 800);
    /// ```
    pub fn max_processes(&self) -> usize
    {
        self.max_processes
    }

    /// Maximum number of cycles before a tie is declared
    ///
    /// # Example
    /// ```
    /// use libcw::simulation::CoreBuilder;
    ///
    /// let core = CoreBuilder::new()
    ///     .max_cycles(800)
    ///     .build_and_load(vec![])
    ///     .unwrap();
    ///
    /// assert_eq!(core.max_cycles(), 800);
    /// // TODO: test that tie happens at this number
    /// ```
    pub fn max_cycles(&self) -> usize
    {
        self.max_cycles
    }

    /// Get immutable reference to memory
    pub fn memory(&self) -> &[Instruction]
    {
        &self.memory.as_slice()
    }

    /// Get the number of processes currently running
    ///
    /// # Example
    /// ```
    /// use libcw::simulation::CoreBuilder;
    /// use libcw::redcode::*;
    ///
    /// let imp = vec![
    ///     Instruction {
    ///         op: OpField {
    ///             code: OpCode::Mov,
    ///             mode: OpMode::I
    ///         },
    ///         a: Field {
    ///             offset: 0,
    ///             mode: AddressingMode::Direct,
    ///         },
    ///         b: Field {
    ///             offset: 1,
    ///             mode: AddressingMode::Direct
    ///         }
    ///     },
    /// ];
    ///
    /// let mut core = CoreBuilder::new().build_and_load(vec![
    ///     (0, None, imp.clone()),
    ///     (4000, None, imp.clone())
    ///     ])
    ///     .unwrap();
    ///
    /// assert_eq!(2, core.process_count());
    /// // TODO: test splitting + test terminating
    /// ```
    pub fn process_count(&self) -> usize
    {
        // count length of all local process queues in the global pqueue
        self.process_queue.iter().fold(1, |acc, &(_, ref x)| acc + x.len())
    }

    /// Execute the instrcution in the `Instruction` register
    fn execute(&mut self) -> CoreEvent
    {
        let code = self.ir.op.code;

        match code {
            OpCode::Dat => self.exec_dat(),
            OpCode::Mov => self.exec_mov(),
            OpCode::Add => self.exec_add(),
            OpCode::Sub => self.exec_sub(),
            OpCode::Mul => self.exec_mul(),
            OpCode::Div => self.exec_div(),
            OpCode::Mod => self.exec_mod(),
            OpCode::Jmp => self.exec_jmp(),
            OpCode::Jmz => self.exec_jmz(),
            OpCode::Jmn => self.exec_jmn(),
            OpCode::Djn => self.exec_djn(),
            OpCode::Spl => self.exec_spl(),
            OpCode::Seq => self.exec_seq(),
            OpCode::Sne => self.exec_sne(),
            OpCode::Slt => self.exec_slt(),
            OpCode::Ldp => self.exec_ldp(),
            OpCode::Stp => self.exec_stp(),
            OpCode::Nop => self.exec_nop(),
        }
    }

    ////////////////////////////////////////////////////////////////////////////
    // Address resolution functions
    ////////////////////////////////////////////////////////////////////////////

    /// Calculate the address after adding an offset
    ///
    /// # Arguments
    /// * `base`: base address
    /// * `offset`: distance from base to calculate
    #[inline]
    fn calc_addr_offset(&self, base: Address, offset: Offset) -> Address
    {
        if offset < 0 {
            (base.wrapping_sub(-offset as Address) % self.size() as Address)
        } else {
            (base.wrapping_add(offset as Address) % self.size() as Address)
        }
    }

    /// Get the effective of address of the current `Instruction`. This takes
    /// into account the addressing mode of the field used
    ///
    /// # Arguments
    /// * `use_a_field`: should the A field be used for calculation, or B
    #[inline]
    fn effective_addr(&self, use_a_field: bool) -> Address
    {
        use self::AddressingMode::*;

        // fetch the addressing mode and offset
        let (mode, offset) = {
            let field = if use_a_field { self.ir.a } else { self.ir.b };
            (field.mode, field.offset)
        };

        let direct = self.fetch(self.calc_addr_offset(self.pc, offset));

        match mode {
            Immediate => self.pc,
            Direct => self.calc_addr_offset(self.pc, offset),
            AIndirect
                | AIndirectPreDecrement
                | AIndirectPostIncrement =>
                self.calc_addr_offset(self.pc, direct.a.offset + offset),
            BIndirect
                | BIndirectPreDecrement
                | BIndirectPostIncrement =>
                self.calc_addr_offset(self.pc, direct.b.offset + offset),
        }
    }

    /// Get the effective of address of the current `Instruction`'s A Field
    ///
    /// An alias for `Core::effective_addr(true)`
    fn effective_addr_a(&self) -> Address
    {
        self.effective_addr(true)
    }

    /// Get the effective of address of the current `Instruction`'s A Field
    ///
    /// An alias for `Core::effective_addr(false)`
    fn effective_addr_b(&self) -> Address
    {
        self.effective_addr(false)
    }

    ////////////////////////////////////////////////////////////////////////////
    // Program counter utility functions
    ////////////////////////////////////////////////////////////////////////////

    /// Move the program counter forward
    fn step_pc(&mut self) -> CoreEvent
    {
        self.pc = (self.pc + 1) % self.size() as Address;
        CoreEvent::Stepped
    }

    /// Move the program counter forward twice
    fn skip_pc(&mut self) -> CoreEvent
    {
        self.pc = (self.pc + 2) % self.size() as Address;
        CoreEvent::Skipped
    }

    /// Jump the program counter by an offset
    ///
    /// # Arguments
    /// * `offset`: amount to jump
    fn jump_pc(&mut self, offset: Offset) -> CoreEvent
    {
        self.pc = self.calc_addr_offset(self.pc, offset);
        CoreEvent::Jumped
    }

    /// Move the program counter forward by one and then queue the program
    /// counter onto the current queue
    fn step_and_queue_pc(&mut self) -> CoreEvent
    {
        self.step_pc();
        self.current_queue.push_front(self.pc);
        CoreEvent::Stepped
    }

    /// Move the program counter forward twice and then queue the program
    /// counter onto the current queue
    fn skip_and_queue_pc(&mut self) -> CoreEvent
    {
        self.skip_pc();
        self.current_queue.push_front(self.pc);
        CoreEvent::Skipped
    }

    /// Jump the program counter by an offset and then queue the program
    /// count onto the current queue
    ///
    /// # Arguments
    /// * `offset`: amount to jump by
    fn jump_and_queue_pc(&mut self, offset: Offset) -> CoreEvent
    {
        self.jump_pc(offset);
        self.current_queue.push_front(self.pc);
        CoreEvent::Jumped
    }

    ////////////////////////////////////////////////////////////////////////////
    // Storage and retrieval functions
    ////////////////////////////////////////////////////////////////////////////

    /// Store an `Instruction` in memory
    ///
    /// # Arguments
    /// * `addr`: address to store
    /// * `instr`: instruction to store
    fn store(&mut self, addr: Address, instr: Instruction)
    {
        let mem_size = self.size();
        self.memory[addr as usize % mem_size] = instr;
    }

    /// Store an instruction in a specified pspace
    ///
    /// # Arguments
    /// * `pin`: programs pin, used as a lookup key
    /// * `addr`: address in the pspace to store
    /// * `instr`: instruction to store
    fn store_pspace(&mut self, pin: Pin, addr: Address, instr: Instruction)
        -> Result<(), ()>
    {
        if let Some(pspace) = self.pspace.get_mut(&pin) {
            let pspace_size = pspace.len();
            pspace[addr as usize % pspace_size] = instr;
            Ok(())
        } else {
            Err(())
        }
    }

    /// Store an `Instruction` into the memory location pointed at by the A
    /// field of the instruction loaded into the instruction register
    ///
    /// # Arguments
    /// * `instr`: `Instruction` to store
    fn store_effective_a(&mut self, instr: Instruction)
    {
        let eff_addr = self.effective_addr_a();
        self.store(eff_addr, instr)
    }

    /// Store an `Instruction` into the memory location pointed at by the B
    /// field of the instruction loaded into the instruction register
    ///
    /// # Arguments
    /// * `instr`: `Instruction` to store
    fn store_effective_b(&mut self, instr: Instruction)
    {
        let eff_addr = self.effective_addr_b();
        self.store(eff_addr, instr)
    }

    /// Fetch copy of instruction in memory
    ///
    /// # Arguments
    /// * `addr`: adress to fetch
    fn fetch(&self, addr: Address) -> Instruction
    {
        self.memory[addr as usize % self.size()]
    }

    /// Fetch an instruction from a programs private storage
    ///
    /// # Arguments
    /// * `pin`: pin of program, used as lookup key
    /// * `addr`: address of pspace to access
    fn fetch_pspace(&self, pin: Pin, addr: Address) -> Result<Instruction, ()>
    {
        if let Some(pspace) = self.pspace.get(&pin) {
            Ok(pspace[addr as usize % pspace.len()])
        } else {
            Err(())
        }
    }

    /// Fetch copy of instruction pointed at by the A field of the instruction
    /// loaded into the instruction register
    fn fetch_effective_a(&self) -> Instruction
    {
        self.fetch(self.effective_addr_a())
    }

    /// Fetch copy of instruction pointed at by the B field of the instruction
    /// loaded into the instruction register
    fn fetch_effective_b(&self) -> Instruction
    {
        self.fetch(self.effective_addr_b())
    }

    ////////////////////////////////////////////////////////////////////////////
    // Instruction execution functions
    ////////////////////////////////////////////////////////////////////////////

    /// Execute `dat` instruction
    ///
    /// Supported OpModes: None
    fn exec_dat(&self) -> CoreEvent
    {
        CoreEvent::Terminated(self.pid())
    }

    /// Execute `mov` instruction
    ///
    /// Supported OpModes: `A` `B` `AB` `BA` `X` `F` `I`
    fn exec_mov(&mut self) -> CoreEvent
    {
        let a     = self.fetch_effective_a();
        let mut b = self.fetch_effective_b();

        match self.ir.op.mode {
            OpMode::A => b.a = a.a,
            OpMode::B => b.b = a.b,
            OpMode::AB =>b.a = a.b,
            OpMode::BA =>b.b = a.a,
            OpMode::F =>
            {
                b.a = a.a;
                b.b = a.b;
            },
            OpMode::X =>
            {
                b.a = a.b;
                b.b = a.a;
            },
            OpMode::I => b = a
        }

        self.store_effective_b(b);
        self.step_and_queue_pc()
    }

    /// Execute `add` instruction
    ///
    /// Supported OpModes: `A` `B` `AB` `BA` `X` `F`
    fn exec_add(&mut self) -> CoreEvent
    {
        // TODO: math needs to be done modulo core size
        let a     = self.fetch_effective_a();
        let mut b = self.fetch_effective_b();

        match self.ir.op.mode {
            OpMode::A  => b.a.offset = (b.a.offset + a.a.offset) % self.size() as Offset,
            OpMode::B  => b.b.offset = (b.b.offset + a.b.offset) % self.size() as Offset,
            OpMode::BA => b.a.offset = (b.a.offset + a.b.offset) % self.size() as Offset,
            OpMode::AB => b.b.offset = (b.b.offset + a.a.offset) % self.size() as Offset,
            OpMode::F
                | OpMode::I =>
            {
                b.a.offset = (b.a.offset + a.a.offset) % self.size() as Offset;
                b.b.offset = (b.b.offset + a.b.offset) % self.size() as Offset;
            },
            OpMode::X =>
            {
                b.b.offset = (b.b.offset + a.a.offset) % self.size() as Offset;
                b.a.offset = (b.a.offset + a.b.offset) % self.size() as Offset;
            },
        }

        self.store_effective_b(b);
        self.step_and_queue_pc()
    }

    /// Execute `sub` instruction
    ///
    /// Supported OpModes: `A` `B` `AB` `BA` `X` `F`
    fn exec_sub(&mut self) -> CoreEvent
    {
        // TODO: math needs to be done modulo core size
        let a     = self.fetch_effective_a();
        let mut b = self.fetch_effective_b();

        match self.ir.op.mode {
            OpMode::A => b.a.offset -= a.a.offset,
            OpMode::B => b.b.offset -= a.b.offset,
            OpMode::BA =>b.a.offset -= a.b.offset,
            OpMode::AB =>b.b.offset -= a.a.offset,
            OpMode::F
                | OpMode::I =>
            {
                b.a.offset -= a.a.offset;
                b.b.offset -= a.b.offset;
            },
            OpMode::X =>
            {
                b.b.offset -= a.a.offset;
                b.a.offset -= a.b.offset;
            },
        }

        self.store_effective_b(b);
        self.step_and_queue_pc()
    }

    /// Execute `mul` instruction
    ///
    /// Supported OpModes: `A` `B` `AB` `BA` `X` `F`
    fn exec_mul(&mut self) -> CoreEvent
    {
        // TODO: math needs to be done modulo core size
        let a     = self.fetch_effective_a();
        let mut b = self.fetch_effective_b();

        match self.ir.op.mode {
            OpMode::A => b.a.offset *= a.a.offset,
            OpMode::B => b.b.offset *= a.b.offset,
            OpMode::BA =>b.a.offset *= a.b.offset,
            OpMode::AB =>b.b.offset *= a.a.offset,
            OpMode::F
                | OpMode::I =>
            {
                b.a.offset *= a.a.offset;
                b.b.offset *= a.b.offset;
            },
            OpMode::X =>
            {
                b.b.offset *= a.a.offset;
                b.a.offset *= a.b.offset;
            },
        }

        self.store_effective_b(b);
        self.step_and_queue_pc()
    }

    /// Execute `div` instruction
    ///
    /// Supported OpModes: `A` `B` `AB` `BA` `X` `F`
    fn exec_div(&mut self) -> CoreEvent
    {
        // TODO: math needs to be done modulo core size
        // TODO: division by zero needs to kill the process
        let a     = self.fetch_effective_a();
        let mut b = self.fetch_effective_b();

        match self.ir.op.mode {
            OpMode::A => b.a.offset /= a.a.offset,
            OpMode::B => b.b.offset /= a.b.offset,
            OpMode::BA =>b.a.offset /= a.b.offset,
            OpMode::AB =>b.b.offset /= a.a.offset,
            OpMode::F
                | OpMode::I =>
            {
                b.a.offset /= a.a.offset;
                b.b.offset /= a.b.offset;
            },
            OpMode::X =>
            {
                b.b.offset /= a.a.offset;
                b.a.offset /= a.b.offset;
            },
        }

        self.store_effective_b(b);
        self.step_and_queue_pc()
    }

    /// Execute `mod` instruction
    ///
    /// Supported OpModes: `A` `B` `AB` `BA` `X` `F`
    fn exec_mod(&mut self) -> CoreEvent
    {
        // TODO: math needs to be done modulo core size
        // TODO: division by zero needs to kill the process
        let a     = self.fetch_effective_a();
        let mut b = self.fetch_effective_b();

        match self.ir.op.mode {
            OpMode::A => b.a.offset %= a.a.offset,
            OpMode::B => b.b.offset %= a.b.offset,
            OpMode::BA =>b.a.offset %= a.b.offset,
            OpMode::AB =>b.b.offset %= a.a.offset,
            OpMode::F
                | OpMode::I =>
            {
                b.a.offset %= a.a.offset;
                b.b.offset %= a.b.offset;
            },
            OpMode::X =>
            {
                b.b.offset %= a.a.offset;
                b.a.offset %= a.b.offset;
            },
        }

        self.store_effective_b(b);
        self.step_and_queue_pc()
    }

    /// Execute `jmp` instruction
    ///
    /// Supported OpModes: `B`
    fn exec_jmp(&mut self) -> CoreEvent
    {
        match self.ir.a.mode {
            AddressingMode::Immediate
                | AddressingMode::Direct =>
            {
                let offset = self.ir.a.offset;
                self.jump_and_queue_pc(offset);
            }
            // TODO
            _ => unimplemented!()
        };

        CoreEvent::Jumped
    }

    /// Execute `jmz` instruction
    ///
    /// Supported OpModes: `B`
    fn exec_jmz(&mut self) -> CoreEvent
    {
        let b = self.fetch_effective_b();
        let offset = self.ir.a.offset; // TODO: needs to calculate jump offset

        let jump = match self.ir.op.mode {
            OpMode::A
                | OpMode::BA => b.a.offset == 0,
            OpMode::B
                | OpMode::AB => b.b.offset == 0,
            OpMode::F
                | OpMode::I
                | OpMode::X => b.a.offset == 0 && b.b.offset == 0,
        };

        if jump {
            self.jump_and_queue_pc(offset)
        } else {
            self.step_and_queue_pc()
        }
    }

    /// Execute `jmn` instruction
    ///
    /// Supported OpModes: `B`
    fn exec_jmn(&mut self) -> CoreEvent
    {
        let b = self.fetch_effective_b();
        let offset = self.ir.a.offset; // TODO: needs to calculate jump offset

        let jump = match self.ir.op.mode {
            OpMode::A
                | OpMode::BA => b.a.offset != 0,
            OpMode::B
                | OpMode::AB => b.b.offset != 0,
            OpMode::F
                | OpMode::I
                | OpMode::X => b.a.offset != 0 && b.b.offset != 0,
        };

        if jump {
            self.jump_and_queue_pc(offset)
        } else {
            self.step_and_queue_pc()
        }
    }

    /// Execute `djn` instruction
    ///
    /// Supported OpModes: `B`
    fn exec_djn(&mut self) -> CoreEvent
    {
        // predecrement the instruction before checking if its not zero
        let mut b = self.fetch_effective_b();
        match self.ir.op.mode {
            OpMode::A
                | OpMode::BA => b.a.offset -= 1,
            OpMode::B
                | OpMode::AB => b.b.offset -= 1,
            OpMode::F
                | OpMode::I
                | OpMode::X =>
            {
                b.a.offset -= 1;
                b.b.offset -= 1;
            }
        }
        self.store_effective_b(b);

        self.exec_jmn()
    }

    /// Execute `spl` instruction
    ///
    /// Supported OpModes: `B`
    fn exec_spl(&mut self) -> CoreEvent
    {
        if self.process_count() < self.max_processes(){
            let target = self.effective_addr_a();
            self.current_queue.push_front(target);

            self.step_and_queue_pc();
            CoreEvent::Split
        } else {
            self.step_and_queue_pc()
        }
    }

    /// Execute `seq` instruction
    ///
    /// Supported OpModes: `A` `B` `AB` `BA` `X` `F` `I`
    fn exec_seq(&mut self) -> CoreEvent
    {
        let a = self.fetch_effective_a();
        let b = self.fetch_effective_b();

        let skip = match self.ir.op.mode {
            OpMode::A       => a.a.offset == b.b.offset,
            OpMode::B       => a.b.offset == b.b.offset,
            OpMode::BA      => a.a.offset == b.b.offset,
            OpMode::AB      => a.b.offset == b.a.offset,
            OpMode::X       => a.b.offset == b.a.offset &&
                               a.a.offset == b.b.offset,
            OpMode::F
                | OpMode::I => a.a.offset == b.a.offset &&
                               a.b.offset == b.b.offset,
        };

        if skip { self.skip_and_queue_pc() } else { self.step_and_queue_pc() }
    }

    /// Execute `sne` instruction
    ///
    /// Supported OpModes: `A` `B` `AB` `BA` `X` `F` `I`
    fn exec_sne(&mut self) -> CoreEvent
    {
        let a = self.fetch_effective_a();
        let b = self.fetch_effective_b();

        let skip = match self.ir.op.mode {
            OpMode::A       => a.a.offset != b.b.offset,
            OpMode::B       => a.b.offset != b.b.offset,
            OpMode::BA      => a.a.offset != b.b.offset,
            OpMode::AB      => a.b.offset != b.a.offset,
            OpMode::X       => a.b.offset != b.a.offset &&
                               a.a.offset != b.b.offset,
            OpMode::F
                | OpMode::I => a.a.offset != b.a.offset &&
                               a.b.offset != b.b.offset,
        };

        if skip { self.skip_and_queue_pc() } else { self.step_and_queue_pc() }
    }

    /// Execute `slt` instruction
    ///
    /// Supported OpModes: `A` `B` `AB` `BA` `X` `F` `I`
    fn exec_slt(&mut self) -> CoreEvent
    {
        let a = self.fetch_effective_a();
        let b = self.fetch_effective_b();

        let skip = match self.ir.op.mode {
            OpMode::A       => a.a.offset < b.b.offset,
            OpMode::B       => a.b.offset < b.b.offset,
            OpMode::BA      => a.a.offset < b.b.offset,
            OpMode::AB      => a.b.offset < b.a.offset,
            OpMode::X       => a.b.offset < b.a.offset &&
                               a.a.offset < b.b.offset,
            OpMode::F
                | OpMode::I => a.a.offset < b.a.offset &&
                               a.b.offset < b.b.offset,
        };

        if skip { self.skip_and_queue_pc() } else { self.step_and_queue_pc() }
    }

    /// Execute `ldp` instruction
    ///
    /// Supported OpModes: `A` `B` `AB` `BA` `X` `F` `I`
    fn exec_ldp(&mut self) -> CoreEvent
    {
        unimplemented!();
    }

    /// Execute `stp` instruction
    ///
    /// Supported OpModes: `A` `B` `AB` `BA` `X` `F` `I`
    fn exec_stp(&mut self) -> CoreEvent
    {
        unimplemented!();
    }

    /// Execute 'nop' instruction
    fn exec_nop(&mut self) -> CoreEvent
    {
        self.step_and_queue_pc()
    }
}

