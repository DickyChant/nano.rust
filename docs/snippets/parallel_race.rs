// A histogram filled in parallel by sharing one mutable accumulator across
// rayon threads. This is a data race -- and the compiler refuses to build it.
// (Inert snippet: lives under docs/, not in any crate, so it never breaks CI.
//  The screencast copies it into nano-io/examples/ to show the rejection live.)
use std::cell::RefCell;
use std::rc::Rc;
use rayon::prelude::*;

fn main() {
    let hist = Rc::new(RefCell::new(0u64)); // one shared histogram bin
    (0..1_000_000).into_par_iter().for_each(|_| {
        *hist.borrow_mut() += 1; // <- concurrent mutation: a data race
    });
    println!("{}", hist.borrow());
}
