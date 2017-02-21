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
        if let Ok((ft, ticket)) = self.get_ticket(&process_id) {
            return Ok((ft, ticket));
        }

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


#[cfg(test)]
mod tests {
    use ::*;
    use string::*;
    use super::*;

    /*
    interesting test cases:
    for each initial condition in [no ticket in queue, ticket in queue]:
        Queue::get_ticket()
        Queue::get_tickets()
        Queue::join()
        Queue::leave() after join
        Queue::leave() before join
    */

    #[test]
    pub fn test_queue_no_row_get() {
        let q = Queue::new();
        assert!(q.get_ticket("foo").is_err());
    }

    #[test]
    pub fn test_queue_row_get() {
        let q = Queue::new();
        let (ft, tok) = q.join_queue(s("foo")).expect("join");
        assert_eq!(ft, 1);
        assert_eq!(&tok.process_id, "foo");
        assert_eq!(tok.counter, 0);
        assert_eq!(tok.position, 0);

        let (ft, tok2) = q.get_ticket("foo").expect("get");
        assert_eq!(ft, 1);
        assert_eq!(tok2, tok);
    }

    #[test]
    pub fn test_queue_no_row_get_all() {
        let q = Queue::new();
        let (ft, toks) = q.get_tickets().expect("get all");
        assert_eq!(ft, 0);
        assert_eq!(toks, vec![]);
    }

    #[test]
    pub fn test_queue_row_get_all() {
        let q = Queue::new();
        let (ft, tok) = q.join_queue(s("foo")).expect("join");
        assert_eq!(ft, 1);
        assert_eq!(&tok.process_id, "foo");
        assert_eq!(tok.counter, 0);
        assert_eq!(tok.position, 0);

        let (ft, toks) = q.get_tickets().expect("get all");
        assert_eq!(ft, 1);
        assert_eq!(toks, vec![tok]);
    }

    #[test]
    pub fn test_queue_no_row_join() {
        let q = Queue::new();
        let (ft, tok) = q.join_queue(s("foo")).expect("join");
        assert_eq!(ft, 1);
        assert_eq!(&tok.process_id, "foo");
        assert_eq!(tok.counter, 0);
        assert_eq!(tok.position, 0);
    }

    #[test]
    pub fn test_queue_same_row_join() {
        let q = Queue::new();
        let (ft, tok) = q.join_queue(s("foo")).expect("join");
        assert_eq!(ft, 1);
        assert_eq!(&tok.process_id, "foo");
        assert_eq!(tok.counter, 0);
        assert_eq!(tok.position, 0);

        let (ft, tok2) = q.join_queue(s("foo")).expect("join");
        assert_eq!(ft, 1);
        assert_eq!(tok2, tok);
    }

    #[test]
    pub fn test_queue_different_row_join() {
        let q = Queue::new();
        let (ft, tok) = q.join_queue(s("foo")).expect("join");
        assert_eq!(ft, 1);
        assert_eq!(&tok.process_id, "foo");
        assert_eq!(tok.counter, 0);
        assert_eq!(tok.position, 0);

        let (ft, tok) = q.join_queue(s("bar")).expect("join");
        assert_eq!(ft, 2);
        assert_eq!(&tok.process_id, "bar");
        assert_eq!(tok.counter, 1);
        assert_eq!(tok.position, 1);
    }

    #[test]
    pub fn test_queue_no_row_leave() {
        let q = Queue::new();
        assert!(q.leave_queue("foo").is_err());
    }

    #[test]
    pub fn test_queue_row_leave() {
        let q = Queue::new();
        let (ft, _tok) = q.join_queue(s("foo")).expect("join");
        assert_eq!(ft, 1);

        let ft = q.leave_queue("foo").expect("leave");
        assert_eq!(ft, 2);
    }

    #[test]
    pub fn test_queue_get_after_leave() {
        let q = Queue::new();
        let (ft, tok) = q.join_queue(s("foo")).expect("join");
        assert_eq!(ft, 1);
        assert_eq!(tok.position, 0);
  
        let (ft, tok) = q.join_queue(s("bar")).expect("join");
        assert_eq!(ft, 2);
        assert_eq!(tok.position, 1);

        let ft = q.leave_queue("foo").expect("leave");
        assert_eq!(ft, 3);

        let (ft, tok) = q.get_ticket("bar").expect("get");
        assert_eq!(ft, 3);
        assert_eq!(tok.position, 0);
        assert_eq!(&tok.process_id, "bar");
        assert_eq!(tok.counter, 1);
    }
}