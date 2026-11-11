//! Scope only constructor selection, including oracle calls made through actions.
//! Each thread restores its previous choice on normal return, errors and panic.
use std::cell::Cell;
thread_local! {static LITERAL: Cell<bool> = const {Cell::new(false)};}
struct Scope(bool);
impl Drop for Scope {
    fn drop(&mut self) {
        LITERAL.with(|flag| flag.set(self.0));
    }
}
pub(crate) fn active() -> bool {
    LITERAL.with(Cell::get)
}
pub(crate) fn literal<T>(build: impl FnOnce() -> crate::Result<T>) -> crate::Result<T> {
    let previous = LITERAL.with(|flag| flag.replace(true));
    let _scope = Scope(previous);
    build()
}
