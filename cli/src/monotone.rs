
#[macro_use]
extern crate error_chain;
extern crate rusoto;
extern crate env_logger;
extern crate monotone;
extern crate clap;
#[macro_use]
extern crate log;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;

pub mod error;

use std::time::Duration;
use std::str::FromStr;
use std::collections::BTreeMap;
use rusoto::{DefaultCredentialsProvider, Region, ProvideAwsCredentials, DispatchSignedRequest};
use rusoto::dynamodb::*;
use rusoto::default_tls_client;
use error::*;
use monotone::*;
use monotone::string::*;
use monotone::aws::dynamodb::*;
use monotone::aws::counter::*;
use monotone::aws::queue::*;

use clap::{Arg, App, SubCommand, ArgMatches};

#[derive(Serialize, Deserialize)]
pub struct CounterValue {
    pub id: String,
    pub value: u64,
    pub region: String,
    pub table: String,
}

#[derive(Serialize, Deserialize)]
pub struct QueueTicketListOutput {
    pub id: String,
    pub region: String,
    pub table: String,
    pub fencing_token: u64,
    pub tickets: Vec<QueueTicket>,
}

#[derive(Serialize, Deserialize)]
pub struct QueueTicketEmptyOutput {
    pub id: String,
    pub region: String,
    pub table: String,
    pub fencing_token: u64,
}

#[derive(Serialize, Deserialize)]
pub struct QueueTicketOutput {
    pub id: String,
    pub region: String,
    pub table: String,
    pub fencing_token: u64,
    pub ticket: QueueTicket,
}

#[derive(Serialize, Deserialize)]
pub struct QueueTicket {
    pub process_id: String,
    pub counter: u64,
    pub position: usize,
    pub tags: BTreeMap<String, String>,
}

fn main() {
    env_logger::init().expect("env_logger::init");

    match run() {
        Err(Error(ErrorKind::MissingArgument(s), _)) => {
            error!("Missing required argument {}\n", s);
            print_help().expect("help");
            std::process::exit(1);
        },
        e => {
            e.expect("error");
        }
    }
}

pub fn run() -> Result<()> {
    let matches = parse_args();

    let provider = DefaultCredentialsProvider::new()?;
    let region = Region::from_str(matches.value_of("region").unwrap_or("eu-west-1"))?;
    let client = DynamoDbClient::new(default_tls_client()?, provider, region);

    match matches.subcommand_name() {
        Some("counter") => {
            let sub_matches = matches.subcommand_matches("counter").unwrap();

            run_counter(region, client, &matches, &sub_matches)?;
        },
        Some("queue") => {
            let sub_matches = matches.subcommand_matches("queue").unwrap();

            run_queue(region, client, &matches, &sub_matches)?;
        },
        Some(c) => {
            error!("Unrecognised subcommand: {}\n", c);
            print_help()?;
            std::process::exit(1);
        },
        None => {
            error!("No subcommand provided\n");
            print_help()?;
            std::process::exit(1);
        }
    }

    Ok(())
}

pub fn run_counter<'a,P,D>(region: Region, client: DynamoDbClient<P,D>, matches: &ArgMatches<'a>, sub_matches: &ArgMatches<'a>) -> Result<()> where P: ProvideAwsCredentials, D: DispatchSignedRequest {

    let table_name = matches.value_of("table").unwrap_or("Counters");
    let id = matches.value_of("id").ok_or(ErrorKind::MissingArgument(s("id")))?;

    match sub_matches.subcommand_name() {
        Some("get")  => {
            create_table_if_needed(&client, table_name, 1, 1)?;
            wait_for_table(&client, table_name)?;

            let counter = Counter::new(client, table_name, id, Duration::from_millis(100));

            let value = counter.get_value()?;

            let result = CounterValue {
                id: s(id),
                region: region.to_string(),
                value: value,
                table: s(table_name),
            };

            println!("{}", serde_json::to_string_pretty(&result)?);
        },
        Some("next") => {
            create_table_if_needed(&client, table_name, 1, 1)?;
            wait_for_table(&client, table_name)?;

            let counter = Counter::new(client, table_name, id, Duration::from_millis(100));

            let value = counter.next_value()?;

            let result = CounterValue {
                id: s(id),
                region: region.to_string(),
                value: value,
                table: s(table_name),
            };

            println!("{}", serde_json::to_string_pretty(&result)?);
        },
        Some("rm") => {
            create_table_if_needed(&client, table_name, 1, 1)?;
            wait_for_table(&client, table_name)?;

            let counter = Counter::new(client, table_name, id, Duration::from_millis(100));

            counter.remove()?;
        },
        Some(c) => {
            error!("Unrecognised subcommand: {}\n", c);
            print_help()?;
            std::process::exit(1);
        },
        None => {
            error!("No subcommand provided\n");
            print_help()?;
            std::process::exit(1);
        }
    }

    Ok(())
}

pub fn run_queue<'a,P,D>(region: Region, client: DynamoDbClient<P,D>, matches: &ArgMatches<'a>, sub_matches: &ArgMatches<'a>) -> Result<()> where P: ProvideAwsCredentials, D: DispatchSignedRequest {

    let table_name = matches.value_of("table").unwrap_or("Counters");
    let id = matches.value_of("id").ok_or(ErrorKind::MissingArgument(s("id")))?;

    match sub_matches.subcommand_name() {
        Some("get")  => {
            let process_id = sub_matches.value_of("process_id").ok_or(ErrorKind::MissingArgument(s("process")))?;

            create_table_if_needed(&client, table_name, 1, 1)?;
            wait_for_table(&client, table_name)?;

            let queue = Queue::new(client, table_name, id, Duration::from_millis(100));

            let (version, ticket) = queue.get_ticket(process_id)?;

            let result = QueueTicketOutput {
                id: s(id),
                region: region.to_string(),
                table: s(table_name),
                fencing_token: version,
                ticket: QueueTicket {
                    process_id: s(ticket.process_id),
                    counter: ticket.counter,
                    position: ticket.position,
                    tags: ticket.tags,
                }
            };

            println!("{}", serde_json::to_string_pretty(&result)?);
        },
        Some("list") => {
            create_table_if_needed(&client, table_name, 1, 1)?;
            wait_for_table(&client, table_name)?;

            let queue = Queue::new(client, table_name, id, Duration::from_millis(100));

            let (version, tickets) = queue.get_tickets()?;

            let mut ticket_list = vec![];
            for t in tickets {
                ticket_list.push(QueueTicket {
                    process_id: s(t.process_id),
                    counter: t.counter,
                    position: t.position,
                    tags: t.tags,
                });
            }

            let result = QueueTicketListOutput {
                id: s(id),
                region: region.to_string(),
                table: s(table_name),
                fencing_token: version,
                tickets: ticket_list,
            };

            println!("{}", serde_json::to_string_pretty(&result)?);
        },
        Some("join") => {
            let join_matches = sub_matches.subcommand_matches("join").unwrap();

            let tags = join_matches.values_of("tag").map(|values| parse_tags(values)).unwrap_or(Ok(BTreeMap::new()))?;

            let process_id = sub_matches.value_of("process_id").ok_or(ErrorKind::MissingArgument(s("process")))?;

            create_table_if_needed(&client, table_name, 1, 1)?;
            wait_for_table(&client, table_name)?;

            let queue = Queue::new(client, table_name, id, Duration::from_millis(100));

            let (version, ticket) = queue.join_queue(s(process_id), tags)?;

            let result = QueueTicketOutput {
                id: s(id),
                region: region.to_string(),
                table: s(table_name),
                fencing_token: version,
                ticket: QueueTicket {
                    process_id: s(ticket.process_id),
                    counter: ticket.counter,
                    position: ticket.position,
                    tags: ticket.tags,
                }
            };

            println!("{}", serde_json::to_string_pretty(&result)?);
        },
        Some("leave") => {
            let process_id = sub_matches.value_of("process_id").ok_or(ErrorKind::MissingArgument(s("process")))?;

            create_table_if_needed(&client, table_name, 1, 1)?;
            wait_for_table(&client, table_name)?;

            let queue = Queue::new(client, table_name, id, Duration::from_millis(100));
            
            let version = queue.leave_queue(process_id)?;

            let result = QueueTicketEmptyOutput {
                id: s(id),
                region: region.to_string(),
                table: s(table_name),
                fencing_token: version
            };

            println!("{}", serde_json::to_string_pretty(&result)?);
        },
        Some("rm") => {
            create_table_if_needed(&client, table_name, 1, 1)?;
            wait_for_table(&client, table_name)?;

            let queue = Queue::new(client, table_name, id, Duration::from_millis(100));
            
            queue.remove()?;
        },
        Some(c) => {
            error!("Unrecognised subcommand: {}\n", c);
            print_help()?;
            std::process::exit(1);
        },
        None => {
            error!("No subcommand provided\n");
            print_help()?;
            std::process::exit(1);
        }
    }

    Ok(())
}

pub fn parse_tags<'a, I>(tags: I) -> Result<BTreeMap<String, String>> where I: Iterator<Item=&'a str> {
    let mut result = BTreeMap::new();
    for t in tags {
        let parts = t.splitn(2, "=").collect::<Vec<&'a str>>();
        if parts.len() == 2 {
            result.insert(s(parts[0]), s(parts[1]));
        } else {
            bail!(ErrorKind::InvalidTag(s(t)));
        }
    }
    Ok(result)
}

pub fn clap_app<'a,'b>() -> App<'a,'b> {
    App::new("aws-counter")
        .version("0.1")
        .author("Chris Dawes <cmsd2@cantab.net>")
        .about("Count things atomically and monotonically")
        .arg(Arg::with_name("region")
            .short("r")
            .long("region")
            .value_name("REGION")
            .help("AWS Region to use for DynamoDB")
            .takes_value(true))
        .arg(Arg::with_name("table")
            .short("t")
            .long("table")
            .value_name("TABLE")
            .help("AWS DynamoDB table")
            .takes_value(true))
        .arg(Arg::with_name("id")
            .short("i")
            .long("id")
            .value_name("COUNTER_ID")
            .help("ID of the counter to manage")
            .takes_value(true))
        .subcommand(SubCommand::with_name("counter")
            .subcommand(SubCommand::with_name("get")
                .about("Get the value of the counter")
                .version("0.1")
                )
            .subcommand(SubCommand::with_name("next")
                .about("Increment and get the value of the counter")
                .version("0.1")
                )
            .subcommand(SubCommand::with_name("rm")
                .about("Remove the counter from the table")
                .version("0.1")
                )
        )
        .subcommand(SubCommand::with_name("queue")
            .arg(Arg::with_name("process_id")
                .short("p")
                .long("process")
                .value_name("PROCESS_ID")
                .help("ID of the process")
                .takes_value(true))
            .subcommand(SubCommand::with_name("get")
                .about("Get the position in the queue for the process id")
                .version("0.1")
                )
            .subcommand(SubCommand::with_name("list")
                .about("Get the processes in the queue")
                .version("0.1")
                )
            .subcommand(SubCommand::with_name("join")
                .about("Add the process id to the back of the queue")
                .version("0.1")
                .arg(Arg::with_name("tag")
                    .short("t")
                    .long("tag")
                    .value_name("TAG")
                    .help("Tag value")
                    .takes_value(true)
                    .multiple(true)
                    )
                )
            .subcommand(SubCommand::with_name("leave")
                .about("Remove the process id from the queue")
                .version("0.1")
                )
            .subcommand(SubCommand::with_name("rm")
                .about("Remove the queue from the table")
                .version("0.1")
                )
        )
}

pub fn print_help() -> Result<()> {
    clap_app().print_help()?;

    Ok(())
}

pub fn parse_args<'a>() -> ArgMatches<'a> {
    clap_app().get_matches()
}