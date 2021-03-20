
[![Crates.io](https://img.shields.io/crates/v/taskwait)](https://crates.io/crates/taskwait)
[![docs.rs](https://img.shields.io/docsrs/taskwait)](https://docs.rs/taskwait)
![Build](https://github.com/staticgc/taskwait/actions/workflows/rust.yml/badge.svg)


# taskwait

Runtime agnostic way of waiting for async tasks.

# Features

* Done: Support for golang's `WaitGroup.Add` & `WaitGroup.Done`
* Done: Support for RAII based `done()`ing the task i.e. calling `done()` on drop.
* Done: Mixing of both `add`, `done` and RAII semantics.
* Done: Reuse the same taskgroup for multiple checkpoints.

# Example 

## Using add & done

```rust
use taskwait::TaskGroup;
 
let tg = TaskGroup::();
for _ in 0..10 {
    tg.add();
    let tg_c = tg.clone();
    tokio::spawn(async move{
        ...
        tg_c.done();
    })
}
tg.wait().await;
```

## Using RAII with add_work

```rust
use taskwait::TaskGroup;
 
let tg = TaskGroup::();
for _ in 0..10 {
    let work = tg.add_work(1); // Increment counter
    tokio::spawn(async move{
        let _work = work; // done() will be called when this is dropped
        ...
    })
}
tg.wait().await;
```
