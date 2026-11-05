//! The spike's workspaces (flick-d330): numbered flat lists of window ids, one of them focused.
//! No split tree yet; flick-ac1d puts `tree` and `workspace` in their place.

use crate::platform::winman::WinId;

/// What focusing another workspace shows and hides.
#[derive(Debug, PartialEq, Eq)]
pub struct Switch {
    pub show: Vec<WinId>,
    pub hide: Vec<WinId>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Spaces {
    lists: Vec<Vec<WinId>>,
    /// Index into `lists`: 0 is workspace 1.
    focused: usize,
}

impl Spaces {
    /// `count` empty workspaces (at least one), the first focused.
    pub fn new(count: usize) -> Spaces {
        Spaces { lists: vec![vec![]; count.max(1)], focused: 0 }
    }

    pub fn count(&self) -> usize {
        self.lists.len()
    }

    /// The focused workspace's index (0 is workspace 1).
    pub fn focused(&self) -> usize {
        self.focused
    }

    /// Workspace `i`'s windows, oldest first.
    pub fn windows(&self, i: usize) -> &[WinId] {
        &self.lists[i]
    }

    /// The index of the workspace that holds `id`.
    pub fn of(&self, id: WinId) -> Option<usize> {
        self.lists.iter().position(|l| l.contains(&id))
    }

    /// Put `id` on the focused workspace, unless a workspace has it.
    pub fn add(&mut self, id: WinId) {
        if self.of(id).is_none() {
            self.lists[self.focused].push(id);
        }
    }

    pub fn remove(&mut self, id: WinId) {
        self.lists.iter_mut().for_each(|l| l.retain(|&w| w != id));
    }

    /// Focus workspace `to`. None when it is focused already or does not exist.
    pub fn switch(&mut self, to: usize) -> Option<Switch> {
        if to == self.focused || to >= self.lists.len() {
            return None;
        }
        let hide = self.lists[self.focused].clone();
        self.focused = to;
        Some(Switch { show: self.lists[to].clone(), hide })
    }

    /// Move every window onto the focused workspace.
    pub fn gather(&mut self) {
        let all: Vec<WinId> = self.lists.iter_mut().flat_map(std::mem::take).collect();
        self.lists[self.focused] = all;
    }

    /// Keep `count` workspaces (at least one). The windows of dropped ones move to the focused
    /// workspace, which moves to the last one kept when it is dropped itself.
    pub fn resize(&mut self, count: usize) {
        let count = count.max(1);
        if count >= self.lists.len() {
            self.lists.resize(count, vec![]);
            return;
        }
        let dropped: Vec<WinId> = self.lists.drain(count..).flatten().collect();
        self.focused = self.focused.min(count - 1);
        self.lists[self.focused].extend(dropped);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_join_the_focused_workspace_once() {
        let s = Spaces::new(0);
        assert_eq!(s.count(), 1);
        let mut s = Spaces::new(3);
        s.add(1);
        s.add(1);
        assert_eq!(s.switch(2), Some(Switch { show: vec![], hide: vec![1] }));
        s.add(2);
        s.add(1);
        assert_eq!((s.of(1), s.of(2), s.of(3)), (Some(0), Some(2), None));
        assert_eq!(s.windows(2), [2]);
        s.remove(1);
        assert_eq!(s.of(1), None);
    }

    #[test]
    fn a_switch_shows_the_new_workspace_and_hides_the_old() {
        let mut s = Spaces::new(2);
        s.add(1);
        assert_eq!(s.switch(0), None);
        assert_eq!(s.switch(2), None);
        s.switch(1);
        s.add(2);
        assert_eq!(s.switch(0), Some(Switch { show: vec![1], hide: vec![2] }));
        assert_eq!(s.focused(), 0);
    }

    #[test]
    fn gather_and_resize_keep_every_window() {
        let mut s = Spaces::new(4);
        s.add(1);
        s.switch(3);
        s.add(4);
        s.resize(6);
        assert_eq!((s.count(), s.of(4)), (6, Some(3)));
        // Workspace 4 (focused) is dropped: its windows and focus go to workspace 2.
        s.resize(2);
        assert_eq!((s.count(), s.focused(), s.of(4), s.of(1)), (2, 1, Some(1), Some(0)));
        s.resize(0);
        assert_eq!((s.count(), s.windows(0)), (1, &[1, 4][..]));
        let mut s = Spaces::new(3);
        s.add(1);
        s.switch(1);
        s.add(2);
        s.gather();
        assert_eq!((s.windows(0), s.windows(1)), (&[][..], &[1, 2][..]));
    }
}
