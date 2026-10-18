use crustdb_core::model::ConnectionId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dialog {
    DeleteConnection { id: ConnectionId, name: String },
}

#[derive(Debug, Default)]
pub struct DialogStack(Vec<Dialog>);

impl DialogStack {
    pub fn push(&mut self, d: Dialog) {
        self.0.push(d);
    }

    pub fn pop(&mut self) -> Option<Dialog> {
        self.0.pop()
    }

    pub fn top(&self) -> Option<&Dialog> {
        self.0.last()
    }
}
