# taskwait

Runtime agnostic way of waiting for async tasks.

# Features

* Done: Support for golang's `WaitGroup.Add` & `WaitGroup.Done`
* Done: Support for RAII based `done()`ing the task i.e. calling `done()` on drop.
* Done: Mixing of both `add`, `done` and RAII semantics.
* Todo: Reuse the same taskgroup for multiple checkpoints.

# Example 

## Using add & done

```rust
use taskwait::TaskGroup;
 
let tg = TaskGroup::();
for _ in 0..10 {
    tg.add(1);
    let tg_c = tg.clone();
    tokio::spawn(async move{
        ...
        tg_c.done();
    })
}
tg.wait().await;
```

## Using add & auto_work

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