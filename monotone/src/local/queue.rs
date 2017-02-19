use std::sync::{Arc, Mutex};

use ::*;
use ::error::*;

#[derive(Debug)]
struct QueueTicket {
    pub process_id: String,
    pub counter: u64,
}

impl QueueTicket {
    pub fn new(process_id: String, counter: u64) -> QueueTicket {
        QueueTicket {
            process_id: process_id,
            counter: counter,
        }
    }
}

#[derive(Debug)]
struct QueueInner {
    items: Vec<QueueTicket>,
    version: u64,
    counter: u64,
}

impl QueueInner {
    pub fn new() -> QueueInner {
        QueueInner {
            items: vec![],
            counter: 0,
            version: 0,
        }
    }

    fn join_queue(&mut self, process_id: String) -> Result<(u64, Ticket)> {
        self.version += 1;

        let position = self.items.len();
        let counter = self.counter;
        let ticket = QueueTicket::new(process_id.clone(), counter);
        
        self.items.push(ticket);
        self.counter += 1;

        Ok((self.version, Ticket::new(process_id, counter, position)))
    }

    fn leave_queue(&mut self, process_id: &str) -> Result<u64> {
        if let Some(pos) = self.items.iter().position(|t| t.process_id == process_id) {
            self.version += 1;

            self.items.remove(pos);
        } else {
            bail!(ErrorKind::NotFound(process_id.to_owned()));
        }

        Ok(self.version)
    }

    fn get_ticket(&self, process_id: &str) -> Result<(u64, Ticket)> {
        self.items
            .iter()
            .enumerate()
            .find(|&(_pos, t)| t.process_id == process_id)
            .map(|(position,t)| (self.version, Ticket::new(t.process_id.clone(), t.counter, position)))
            .ok_or_else(|| ErrorKind::NotFound(process_id.to_owned()).into())
    }

    fn get_tickets(&self) -> Result<(u64, Vec<Ticket>)> {
        Ok((self.version, self.items
            .iter()
            .enumerate()
            .map(|(position,t)| Ticket::new(t.process_id.clone(), t.counter, position))
            .collect()))
    }
}

#[derive(Debug)]
pub struct Queue {
    items: Arc<Mutex<QueueInner>>
}

impl Queue {
    pub fn new() -> Queue {
        Queue {
            items: Arc::new(Mutex::new(QueueInner::new()))
        }
    }
}

impl MonotonicQueue for Queue {
    type Error = Error;
    
    fn join_queue(&self, process_id: String) -> Result<(u64, Ticket)> {
        let mut inner = self.items.lock().unwrap();

        inner.join_queue(process_id)
    }

    fn leave_queue(&self, process_id: &str) -> Result<u64> {
        let mut inner = self.items.lock().unwrap();

        inner.leave_queue(process_id)
    }

    fn get_ticket(&self, process_id: &str) -> Result<(u64, Ticket)> {
        let inner = self.items.lock().unwrap();

        inner.get_ticket(process_id)
    }

    fn get_tickets(&self) -> Result<(u64, Vec<Ticket>)> {
        let inner = self.items.lock().unwrap();

        inner.get_tickets()
    }
}