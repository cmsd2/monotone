//! `monotone`: monotonic counters and fenced queues on AWS DynamoDB.

use std::collections::BTreeMap;
use std::process::ExitCode;

use aws_sdk_dynamodb::Client;
use clap::{CommandFactory, Parser, Subcommand};
use monotone::dynamodb::{self, Counter, Queue, table};
use monotone::{MonotonicCounter, MonotonicQueue, Tags, Ticket};
use serde::Serialize;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "monotone",
    version,
    about = "Count things atomically and monotonically on AWS DynamoDB",
    long_about = "Count things atomically and monotonically on AWS DynamoDB.\n\n\
                  Credentials come from the standard AWS chain. Set AWS_ENDPOINT_URL to use \
                  DynamoDB Local. Set RUST_LOG=debug for diagnostics on stderr."
)]
struct Cli {
    /// AWS region to use for DynamoDB
    #[arg(
        short,
        long,
        global = true,
        value_name = "REGION",
        default_value = "eu-west-1"
    )]
    region: String,

    /// DynamoDB table holding counters and queues
    #[arg(
        short,
        long,
        global = true,
        value_name = "TABLE",
        default_value = "Counters"
    )]
    table: String,

    /// ID of the counter or queue to manage
    #[arg(short, long, global = true, value_name = "ID")]
    id: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Operate on a monotonic counter
    Counter {
        #[command(subcommand)]
        op: Option<CounterOp>,
    },
    /// Operate on a monotonic queue
    Queue {
        /// ID of the process
        #[arg(
            short = 'p',
            long = "process",
            global = true,
            value_name = "PROCESS_ID"
        )]
        process: Option<String>,

        #[command(subcommand)]
        op: Option<QueueOp>,
    },
}

#[derive(Debug, Subcommand)]
enum CounterOp {
    /// Get the value of the counter
    Get,
    /// Increment and get the value of the counter
    Next,
    /// Remove the counter from the table
    Rm,
}

#[derive(Debug, Subcommand)]
enum QueueOp {
    /// Get the position in the queue for the process
    Get,
    /// List the processes in the queue
    List,
    /// Add the process to the back of the queue
    Join {
        /// Tag to attach to the ticket, as KEY=VALUE. May be repeated.
        #[arg(long = "tag", value_name = "KEY=VALUE")]
        tags: Vec<String>,
    },
    /// Remove the process from the queue
    Leave,
    /// Remove the queue from the table
    Rm,
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("Missing required argument {0}")]
    MissingArgument(&'static str),

    #[error("No subcommand provided")]
    NoSubcommand,

    #[error("invalid tag: {0}")]
    InvalidTag(String),

    #[error(transparent)]
    Monotone(#[from] dynamodb::Error),

    #[error("could not write output: {0}")]
    Json(#[from] serde_json::Error),
}

impl Error {
    /// Usage errors are followed by the help text.
    fn wants_help(&self) -> bool {
        matches!(self, Error::MissingArgument(_) | Error::NoSubcommand)
    }
}

#[derive(Serialize)]
struct CounterValue<'a> {
    id: &'a str,
    value: u64,
    region: &'a str,
    table: &'a str,
}

#[derive(Serialize)]
struct QueueTicket {
    process_id: String,
    counter: u64,
    position: usize,
    tags: Tags,
}

impl From<Ticket> for QueueTicket {
    fn from(t: Ticket) -> Self {
        QueueTicket {
            process_id: t.process_id,
            counter: t.counter,
            position: t.position,
            tags: t.tags,
        }
    }
}

#[derive(Serialize)]
struct QueueOutput<'a> {
    id: &'a str,
    region: &'a str,
    table: &'a str,
    fencing_token: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    ticket: Option<QueueTicket>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tickets: Option<Vec<QueueTicket>>,
}

fn print_json<T: Serialize>(value: &T) -> Result<(), Error> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn parse_tags(raw: &[String]) -> Result<Tags, Error> {
    let mut tags = BTreeMap::new();
    for t in raw {
        match t.split_once('=') {
            Some((k, v)) => {
                tags.insert(k.to_owned(), v.to_owned());
            }
            None => return Err(Error::InvalidTag(t.clone())),
        }
    }
    Ok(tags)
}

/// A validated invocation, built before any network call.
enum Action {
    Counter(CounterOp),
    QueueGet(String),
    QueueList,
    QueueJoin(String, Tags),
    QueueLeave(String),
    QueueRm,
}

fn plan(cli: &mut Cli) -> Result<(String, Action), Error> {
    let command = cli.command.take().ok_or(Error::NoSubcommand)?;
    let id = cli.id.clone().ok_or(Error::MissingArgument("id"))?;
    let action = match command {
        Command::Counter { op } => Action::Counter(op.ok_or(Error::NoSubcommand)?),
        Command::Queue { process, op } => {
            let process = || process.clone().ok_or(Error::MissingArgument("process"));
            match op.ok_or(Error::NoSubcommand)? {
                QueueOp::Get => Action::QueueGet(process()?),
                QueueOp::List => Action::QueueList,
                QueueOp::Join { tags } => {
                    let tags = parse_tags(&tags)?;
                    Action::QueueJoin(process()?, tags)
                }
                QueueOp::Leave => Action::QueueLeave(process()?),
                QueueOp::Rm => Action::QueueRm,
            }
        }
    };
    Ok((id, action))
}

async fn run(mut cli: Cli) -> Result<(), Error> {
    let (id, action) = plan(&mut cli)?;
    let (region, table_name) = (cli.region.as_str(), cli.table.as_str());

    let config = dynamodb::load_config(Some(cli.region.clone())).await;
    let client = Client::new(&config);
    table::create_table_if_needed(&client, table_name, 1, 1).await?;
    table::wait_for_table(&client, table_name).await?;

    let queue = || Queue::new(client.clone(), table_name, &id);
    let queue_output =
        |fencing_token, ticket: Option<Ticket>, tickets: Option<Vec<Ticket>>| QueueOutput {
            id: &id,
            region,
            table: table_name,
            fencing_token,
            ticket: ticket.map(QueueTicket::from),
            tickets: tickets.map(|ts| ts.into_iter().map(QueueTicket::from).collect()),
        };

    match action {
        Action::Counter(op) => {
            let counter = Counter::new(client.clone(), table_name, &id);
            let value = match op {
                CounterOp::Get => counter.get_value().await?,
                CounterOp::Next => counter.next_value().await?,
                CounterOp::Rm => return Ok(counter.remove().await?),
            };
            print_json(&CounterValue {
                id: &id,
                value,
                region,
                table: table_name,
            })
        }
        Action::QueueGet(process) => {
            let (token, ticket) = queue().get_ticket(&process).await?;
            print_json(&queue_output(token, Some(ticket), None))
        }
        Action::QueueList => {
            let (token, tickets) = queue().get_tickets().await?;
            print_json(&queue_output(token, None, Some(tickets)))
        }
        Action::QueueJoin(process, tags) => {
            let (token, ticket) = queue().join_queue(&process, tags).await?;
            print_json(&queue_output(token, Some(ticket), None))
        }
        Action::QueueLeave(process) => {
            let token = queue().leave_queue(&process).await?;
            print_json(&queue_output(token, None, None))
        }
        Action::QueueRm => Ok(queue().remove().await?),
    }
}

fn init_logging() {
    let filter = EnvFilter::try_from_env("RUST_LOG").unwrap_or_else(|_| EnvFilter::new("error"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) if !e.use_stderr() => {
            // --help and --version.
            let _ = e.print();
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            let _ = e.print();
            eprintln!("\n{}", Cli::command().render_help());
            return ExitCode::FAILURE;
        }
    };

    init_logging();

    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            if e.wants_help() {
                eprintln!("\n{}", Cli::command().render_help());
            }
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clap_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn tags_split_on_first_equals() {
        let tags = parse_tags(&["url=http://a=b".into(), "role=zk".into()]).unwrap();
        assert_eq!(tags["url"], "http://a=b");
        assert_eq!(tags["role"], "zk");
        assert!(
            matches!(parse_tags(&["novalue".into()]), Err(Error::InvalidTag(t)) if t == "novalue")
        );
    }
}
