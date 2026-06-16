# OML Task Schema

FerretOS uses [OML](https://github.com/Bilal-Waraich/OML) — a lightweight struct/class/instance DSL — to declare tasks at build time. The `tasks/` directory contains the schema and instance declarations; `build.rs` invokes the OML transpiler to generate Rust into `kernel/src/generated/`.

---

## Schema file: `tasks/task.oml`

```
enum Peripheral {
    string UART0;
    string UART1;
    string GPIO0;
    string GPIO1;
    string SPI0;
    string I2C0;
    string NONE;
}

struct TaskConfig {
    public uint8  priority;
    public uint32 stack_size;
    public uint32 memory_start;
    public uint32 memory_end;
    public uint32 exclusive_cap_mask;
    public uint32 shared_cap_mask;
    public uint32 required_cap_mask;
}
```

### Field reference

| Field | Type | Description |
|-------|------|-------------|
| `priority` | `uint8` | Base scheduling priority. Higher value = higher priority. Used as the tiebreak input when no MIP applies. |
| `stack_size` | `uint32` | Stack buffer size in bytes. Must be a multiple of 16 (RISC-V ABI alignment). |
| `memory_start` | `uint32` | Inclusive start address of the task's private memory region. |
| `memory_end` | `uint32` | Exclusive end address of the task's private memory region. |
| `exclusive_cap_mask` | `uint32` | One-hot bitmask of peripherals this task holds exclusively. Bit `i` = peripheral ID `i`. No two tasks may share a set bit — the boot-time conflict detector halts if they do. |
| `shared_cap_mask` | `uint32` | One-hot bitmask of peripherals this task holds in shared (read-only) mode. Shared access does not create CCG edges. |
| `required_cap_mask` | `uint32` | One-hot bitmask of peripherals this task requires but does not currently hold. Used by the CCG builder: an edge L → H is added when `L.exclusive_cap_mask & H.required_cap_mask != 0`. |

### Peripheral bitmask encoding

```
Bit 0 — UART0   (0x0000_0001)
Bit 1 — UART1   (0x0000_0002)
Bit 2 — GPIO0   (0x0000_0004)
Bit 3 — GPIO1   (0x0000_0008)
Bit 4 — SPI0    (0x0000_0010)
Bit 5 — I2C0    (0x0000_0020)
```

The encoding is one-hot (single bit per peripheral) to allow O(1) conflict detection via bitwise AND.

---

## Instance file: `tasks/demo_tasks.oml`

```
import "task.oml";

instance TASK_L: TaskConfig {
    priority = 1;
    stack_size = 4096;
    memory_start = 0x8008_1000;
    memory_end = 0x8008_2000;
    exclusive_cap_mask = 0x0000_0001;   // holds UART0
    shared_cap_mask = 0;
    required_cap_mask = 0;
}

instance TASK_M: TaskConfig {
    priority = 2;
    stack_size = 4096;
    memory_start = 0x8008_2000;
    memory_end = 0x8008_3000;
    exclusive_cap_mask = 0;
    shared_cap_mask = 0;
    required_cap_mask = 0;
}

instance TASK_H: TaskConfig {
    priority = 3;
    stack_size = 4096;
    memory_start = 0x8008_3000;
    memory_end = 0x8008_4000;
    exclusive_cap_mask = 0;
    shared_cap_mask = 0;
    required_cap_mask = 0x0000_0001;    // requires UART0
}
```

The `instance` keyword is an OML extension that generates a `pub static NAME: TYPE = TYPE { ... };` binding in Rust.

---

## Generated output

OML transpiles `task.oml` → `kernel/src/generated/task_schema.rs`:

```rust
#[derive(Clone, Copy)]
pub struct TaskConfig {
    pub priority: u8,
    pub stack_size: u32,
    pub memory_start: u32,
    pub memory_end: u32,
    pub exclusive_cap_mask: u32,
    pub shared_cap_mask: u32,
    pub required_cap_mask: u32,
}
```

OML transpiles `demo_tasks.oml` → `kernel/src/generated/demo_tasks.rs`:

```rust
pub static TASK_L: TaskConfig = TaskConfig {
    priority: 1,
    stack_size: 4096,
    memory_start: 0x8008_1000,
    memory_end: 0x8008_2000,
    exclusive_cap_mask: 0x0000_0001,
    shared_cap_mask: 0,
    required_cap_mask: 0,
};
// ... TASK_M, TASK_H
```

`kernel/src/generated/bridge.rs` (hand-written, not regenerated) provides the conversion:

```rust
impl TaskConfig {
    pub fn into_descriptor(self, id: u8, stack_base: usize) -> TaskDescriptor {
        TaskDescriptor::with_capabilities(id, self.priority, stack_base, ...)
    }
}
```

`kernel_main` registers tasks as:

```rust
register_task(TASK_L.into_descriptor(0, stack_l_base));
register_task(TASK_M.into_descriptor(1, stack_m_base));
register_task(TASK_H.into_descriptor(2, stack_h_base));
```

---

## Adding a new task

1. Add an `instance` block to `tasks/demo_tasks.oml` (or a new `.oml` file with `import "task.oml"`).
2. Run `OML_BIN=/path/to/oml cargo build` — `build.rs` regenerates `kernel/src/generated/`.
3. Commit the regenerated files so the kernel builds without OML installed.
4. Register the new task in `kernel_main` with `register_task(TASK_NEW.into_descriptor(id, stack_base))`.
5. Update `MAX_TASKS` in `config.rs` if the registry would overflow.

---

## Build-time regeneration

`kernel/build.rs` locates the OML binary via (in order):

1. `OML_BIN` environment variable
2. `oml/target/release/oml` — submodule build
3. `../../OML/target/release/oml` — sibling repository (local dev)
4. `oml` on `PATH`

If no binary is found, the committed generated files are used and a Cargo warning is printed. The kernel always builds without OML installed.
