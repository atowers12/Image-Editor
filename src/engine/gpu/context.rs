//! A headless wgpu device, and turning its asynchronous error reporting into
//! ordinary `Result`s so a GPU failure becomes a CPU fallback, not a panic.

use std::sync::{Arc, Mutex};

use anyhow::{bail, Result};

/// The GPU ran out of memory. Unlike other GPU errors this says nothing is
/// wrong with the device — the work was just too big for it — so callers
/// fall back to the CPU for that job without giving up on the GPU.
#[derive(Debug)]
pub struct OutOfMemory;

impl std::fmt::Display for OutOfMemory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("out of GPU memory")
    }
}

impl std::error::Error for OutOfMemory {}

/// There is no usable GPU at all — as opposed to one that failed.
#[derive(Debug)]
pub struct NoAdapter(pub String);

impl std::fmt::Display for NoAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NoAdapter {}

pub struct Context {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub limits: wgpu::Limits,
    label: String,
    /// The first error reported outside an error scope, or why the device
    /// was lost. Once set, every later operation fails.
    fault: Arc<Mutex<Option<String>>>,
}

impl Context {
    /// Open the best available hardware adapter. With `allow_software`, a
    /// software adapter (WARP on Windows) is accepted when there is no
    /// hardware one — useful for tests, but slower than the CPU pipeline for
    /// the app itself.
    pub fn new(allow_software: bool) -> Result<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });
        let request = |force_fallback_adapter| {
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter,
                compatible_surface: None,
            }))
        };
        let adapter = request(false)
            .or_else(|| allow_software.then(|| request(true)).flatten())
            .ok_or_else(|| NoAdapter("no compatible GPU found".into()))?;
        let info = adapter.get_info();
        if info.device_type == wgpu::DeviceType::Cpu && !allow_software {
            let why = format!("only a software adapter ({}) is available", info.name);
            return Err(NoAdapter(why).into());
        }
        // Ask for everything the adapter offers — notably texture edges past
        // the 8192 default, for full-resolution photos.
        let limits = adapter.limits();
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("photo-editor"),
                required_features: wgpu::Features::empty(),
                required_limits: limits.clone(),
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))?;

        // The default handler panics, which would take the render worker down.
        let fault = Arc::new(Mutex::new(None::<String>));
        let f = fault.clone();
        device.on_uncaptured_error(Box::new(move |e| {
            if let Ok(mut slot) = f.lock() {
                slot.get_or_insert_with(|| format!("GPU error: {e}"));
            }
        }));
        let f = fault.clone();
        device.set_device_lost_callback(move |_, msg| {
            if let Ok(mut slot) = f.lock() {
                slot.get_or_insert_with(|| format!("GPU device lost: {msg}"));
            }
        });

        Ok(Context {
            device,
            queue,
            limits,
            label: format!("{} ({:?})", info.name, info.backend),
            fault,
        })
    }

    /// Adapter name and backend, e.g. "NVIDIA GeForce RTX 3070 (Dx12)".
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Run `f`, turning any validation or out-of-memory error it raises —
    /// or a device fault since — into an `Err`. Running out of memory comes
    /// back as `OutOfMemory`.
    pub fn scoped<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let out = f();
        let validation = pollster::block_on(self.device.pop_error_scope());
        let oom = pollster::block_on(self.device.pop_error_scope());
        if let Some(e) = validation {
            bail!("GPU error: {e}");
        }
        if oom.is_some() {
            return Err(OutOfMemory.into());
        }
        if let Some(e) = self.fault.lock().ok().and_then(|f| f.clone()) {
            bail!(e);
        }
        out
    }
}
