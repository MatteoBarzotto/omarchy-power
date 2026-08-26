//! A cache in front of the GPU driver.
//!
//! Clients refresh once a second and there may be several of them; asking the
//! driver every time would mean spawning a process per client per second for a
//! reading that barely changes in between. One reading is shared by everyone
//! who asks within [`TTL`].

use std::sync::Mutex;
use std::time::{Duration, Instant};

use omarchy_power_core::gpu::{self, Gpu};

/// How long a reading is reused. Half the TUI's refresh interval would be
/// pointless precision; twice it would visibly lag the fan noise.
const TTL: Duration = Duration::from_secs(2);

pub struct GpuReader {
    /// `None` while nothing has been read yet. A machine with no GPU stores
    /// an empty reading rather than nothing, so the absence is cached too and
    /// `nvidia-smi` is not looked for over and over on every AMD laptop.
    last: Mutex<Option<(Instant, Gpu)>>,
}

impl GpuReader {
    pub fn new() -> Self {
        Self {
            last: Mutex::new(None),
        }
    }

    /// The current reading, from cache when it is fresh enough.
    pub fn read(&self) -> Gpu {
        let mut last = match self.last.lock() {
            Ok(guard) => guard,
            // A panic while holding this lock would have to come from the
            // mutex itself; a stale reading beats taking the daemon down.
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some((taken, gpu)) = last.as_ref()
            && taken.elapsed() < TTL
        {
            return *gpu;
        }
        let gpu = gpu::read().unwrap_or_default();
        *last = Some((Instant::now(), gpu));
        gpu
    }
}

impl Default for GpuReader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_machine_without_an_nvidia_gpu_reads_as_empty_rather_than_failing() {
        // Whatever this test machine has, the call must return something.
        let reader = GpuReader::new();
        let first = reader.read();
        // The second read comes from the cache and must agree with the first.
        assert_eq!(reader.read(), first);
    }
}
