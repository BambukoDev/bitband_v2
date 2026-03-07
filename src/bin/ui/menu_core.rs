use heapless::String;

pub const MAX_NAME: usize = 32;
pub const MENU_DEPTH_MAX: usize = 4;

#[derive(Clone)]
pub enum Action {
    RunDuck(String<MAX_NAME>),
    ToggleBluetooth,
    AccessPoint(bool),
    Reboot,
}

#[derive(Clone)]
pub enum MenuAction {
    Enter(&'static dyn MenuSource),
    Trigger(Action),
}

pub trait MenuSource: Sync {
    fn title(&self) -> &str;
    fn len(&self) -> usize;
    fn label(&self, index: usize) -> &str;
    fn action(&self, index: usize) -> MenuAction;
}

pub struct MenuItem {
    pub label: &'static str,
    pub action: MenuAction,
}

pub struct StaticMenu {
    pub title: &'static str,
    pub items: &'static [MenuItem],
}

impl MenuSource for StaticMenu {
    fn title(&self) -> &str {
        self.title
    }

    fn len(&self) -> usize {
        self.items.len()
    }

    fn label(&self, index: usize) -> &str {
        self.items[index].label
    }

    fn action(&self, index: usize) -> MenuAction {
        self.items[index].action.clone()
    }
}
