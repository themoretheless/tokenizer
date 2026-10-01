use std::{
    any::Any,
    fmt,
    rc::{Rc, Weak},
};

/// A non-owning reference to an application-owned object. Script aliases do not
/// keep it alive; the last strong Rust owner determines when it expires.
#[derive(Clone)]
pub struct HostObject {
    name: &'static str,
    object: Weak<dyn Any>,
}
impl HostObject {
    pub fn new<T: Any>(name: &'static str, owner: &Rc<T>) -> Self {
        let erased: Rc<dyn Any> = owner.clone();
        Self {
            name,
            object: Rc::downgrade(&erased),
        }
    }
    pub fn type_name(&self) -> &'static str {
        self.name
    }
    pub fn is_alive(&self) -> bool {
        self.object.strong_count() != 0
    }
    /// Pin a live object for one host operation. A wrong Rust type or expired
    /// owner returns None. The returned Rc keeps the object alive until dropped.
    pub fn upgrade<T: Any>(&self) -> Option<Rc<T>> {
        self.object.upgrade()?.downcast().ok()
    }
}
impl PartialEq for HostObject {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && Weak::ptr_eq(&self.object, &other.object)
    }
}
impl Eq for HostObject {}
impl fmt::Debug for HostObject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostObject")
            .field("type", &self.name)
            .field("alive", &self.is_alive())
            .finish()
    }
}
