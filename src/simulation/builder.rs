//! Utility struct for builder `Mars`s

use std::collections::{VecDeque, HashMap};

use redcode::types::*;
use redcode::traits::Instruction;

use simulation::Mars;
use simulation::LoadResult;

// Mars defaults
const DEFAULT_SIZE: usize          = 8000;
const DEFAULT_PSPACE_SIZE: usize   = 500;
const DEFAULT_MAX_CYCLES: usize    = 80000;
const DEFAULT_MAX_PROCESSES: usize = 8000;
const DEFAULT_MAX_LENGTH: usize    = 100;
const DEFAULT_MIN_DISTANCE: usize  = 100;
const DEFAULT_VERSION: usize       = 80; // FIXME: hmmm

/// Errors that can occur from invalid `MarsBuilder` configuration
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum BuilderError
{
    /// Program is longer than the core allows
    ProgramTooLong,

    /// A provided offset would violate a constraint of the `Mars`
    InvalidOffset
}

/// A `Mars` builder. Provides control over how the `Mars` is
/// configured
#[derive(Debug, Clone)]
pub struct MarsBuilder
{
    /// Size of core's memory buffer
    size:     usize,

    /// Size of each warrior's p-space
    pspace_size:   usize,

    /// Maximum number of cycles before game is considered a draw
    max_cycles:    usize,

    /// Maximum number of processes that can be in the process queue
    max_processes: usize,

    /// Maximum number of instructions a warrior can be comprised of
    max_length:    usize,

    /// Minimum distance between two warriors
    min_distance:  usize,

    /// Mars Version multiplied by 100
    version:       usize,
}

impl MarsBuilder
{
    /// Create a `MarsBuilder` with default parameters
    pub fn new() -> Self
    {
        MarsBuilder {
            size:          DEFAULT_SIZE,
            pspace_size:   DEFAULT_PSPACE_SIZE,
            max_cycles:    DEFAULT_MAX_CYCLES,
            max_processes: DEFAULT_MAX_PROCESSES,
            max_length:    DEFAULT_MAX_LENGTH,
            min_distance:  DEFAULT_MIN_DISTANCE,
            version:       DEFAULT_VERSION
        }
    }

    /// Build a core and load it with specified programs
    pub fn build_and_load<T>(&self, programs: Vec<(Address, Option<Pin>, &Vec<T>)>) 
        -> LoadResult<Mars<T>>
        where T: Instruction
    {
        let mut core = self.build();
        if programs.len() > 0 {
            core.load_batch(programs)?;
        }
        Ok(core)
    }

    /// Build a halted mars
    pub fn build<T>(&self) -> Mars<T>
        where T: Instruction
    {
        // create core resources
        let mem    = vec![T::default(); self.size];
        let pq     = VecDeque::new();
        let pspace = HashMap::new();

        Mars {
            // Runtime data
            memory:        mem,
            cycle:         0,
            process_queue: pq,
            pspace:        pspace,
            halted:        true,
            ir:            Default::default(),
            pc:            0,
            pid:           Default::default(),

            // Load constraints
            max_length:    self.max_length,
            min_distance:  self.min_distance,

            // Mars information(const)
            version:       self.version,
            pspace_size:   self.pspace_size,

            // Runtime constraints
            max_processes: self.max_processes,
            max_cycles:    self.max_cycles,
        }
    }

    /// Size of memory
    ///
    /// # Arguments
    /// * `size`: size of memory
    ///
    /// # Return
    /// `Self`
    pub fn size(&mut self, size: usize) -> &mut Self
    {
        self.size = size;
        self
    }

    /// Size of each warrior's P-space
    ///
    /// # Arguments
    /// * `size`: size of memory
    ///
    /// # Return
    /// `Self`
    pub fn pspace_size(&mut self, size: usize) -> &mut Self
    {
        self.pspace_size = size;
        self
    }

    /// Maximum number of cycles that can elapse before a tie is declared
    ///
    /// # Arguments
    /// * `n`: number of cycles
    ///
    /// # Return
    /// `Self`
    pub fn max_cycles(&mut self, n: usize) -> &mut Self
    {
        self.max_cycles = n;
        self
    }

    /// Maximum number of processes a core can have in it's process queue
    ///
    /// # Arguments
    /// * `n`: number of processes
    ///
    /// # Return
    /// `Self`
    pub fn max_processes(&mut self, n: usize) -> &mut Self
    {
        self.max_processes = n;
        self
    }

    /// Maximum number of instructions allowed in a program
    ///
    /// # Arguments
    /// * `n`: number of instructions
    ///
    /// # Return
    /// `Self`
    pub fn max_length(&mut self, n: usize) -> &mut Self
    {
        self.max_length = n;
        self
    }

    /// Minimum distance between warriors
    ///
    /// # Arguments
    /// * `n`: number of instructions
    ///
    /// # Return
    /// `Self`
    pub fn min_distance(&mut self, n: usize) -> &mut Self
    {
        self.min_distance = n;
        self
    }

    /// Mars version multiplied by 100 (e.g. version 0.8 -> 80)
    ///
    /// # Arguments
    /// * `version`: version number
    ///
    /// # Return
    /// `Self`
    pub fn version(&mut self, version: usize) -> &mut Self
    {
        self.version = version;
        self
    }
}

#[cfg(test)]
mod test_builder
{
    use super::*;
    use redcode::Instruction;

    /// Verify that the `MarsBuilder` creates a halted `Mars` if no programs
    /// were loaded
    #[test]
    fn test_build_mars_is_halted()
    {
        let mars: Mars<Instruction> = MarsBuilder::new().build();
        assert_eq!(true, mars.halted());
    }

    /// Verify that the `MarsBuilder` correctly sets the version in the `Mars`
    #[test]
    fn test_version_set()
    {
        let mars: Mars<Instruction> = MarsBuilder::new()
            .version(890)
            .build();

        assert_eq!(890, mars.version());
    }

    /// Verify that the `MarsBuilder` correctly sets the size in the `Mars`
    #[test]
    fn test_size_set()
    {
        let mars: Mars<Instruction> = MarsBuilder::new()
            .size(890)
            .build();

        assert_eq!(890, mars.size());
    }

    /// Verify that the `MarsBuilder` correctly sets the p-space size in the 
    /// `Mars`
    #[test]
    fn test_pspace_size_set()
    {
        let mars: Mars<Instruction> = MarsBuilder::new()
            .pspace_size(890)
            .build();

        assert_eq!(890, mars.pspace_size());
    }

    /// Verify that the `MarsBuilder` correctly sets the max cycles in the 
    /// `Mars`
    #[test]
    fn test_max_cycles_set()
    {
        let mars: Mars<Instruction> = MarsBuilder::new()
            .max_cycles(890)
            .build();

        assert_eq!(890, mars.max_cycles());
    }

    /// Verify that the `MarsBuilder` correctly sets the max processes in the 
    /// `Mars`
    #[test]
    fn test_max_processes_set()
    {
        let mars: Mars<Instruction> = MarsBuilder::new()
            .max_processes(890)
            .build();

        assert_eq!(890, mars.max_processes());
    }

    /// Verify that the `MarsBuilder` correctly sets the max length in the 
    /// `Mars`
    #[test]
    fn test_max_length_set()
    {
        let mars: Mars<Instruction> = MarsBuilder::new()
            .max_length(890)
            .build();

        assert_eq!(890, mars.max_length());
    }
    
    /// Verify that the `MarsBuilder` correctly sets the minimum distance in the
    /// `Mars`
    #[test]
    fn test_min_distance_set()
    {
        let mars: Mars<Instruction> = MarsBuilder::new()
            .min_distance(890)
            .build();

        assert_eq!(890, mars.min_distance());
    }
}

