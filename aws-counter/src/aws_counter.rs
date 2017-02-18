
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
use rusoto::{DefaultCredentialsProvider, Region};
use rusoto::dynamodb::*;
use rusoto::default_tls_client;
use error::*;
use monotone::*;
use monotone::string::*;
use monotone::aws::dynamodb::*;
use monotone::aws::counter::*;

use clap::{Arg, App, SubCommand, ArgMatches};

#[derive(Serialize, Deserialize)]
pub struct CounterValue {
    pub id: String,
    pub value: u64,
    pub region: String,
    pub table: String,
}

fn main() {
    match run() {
        Err(Error(ErrorKind::MissingArgument(s), _)) => {
            println!("Missing required argument {}\n", s);
            print_help();
            std::process::exit(1);
        },
        e => {
            e.expect("error");
        }
    }
}

pub fn run() -> Result<()> {
    env_logger::init()?;

    let matches = parse_args();

    let region = Region::from_str(matches.value_of("region").unwrap_or("eu-west-1"))?;
    let table_name = matches.value_of("table").unwrap_or("Counters");
    let id = matches.value_of("id").ok_or(ErrorKind::MissingArgument(s("id")))?;

    let provider = DefaultCredentialsProvider::new()?;
    let client = DynamoDbClient::new(default_tls_client()?, provider, region);

    match matches.subcommand_name() {
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
            unimplemented!()
        },
        _ => {
            print_help()?;
            std::process::exit(1);
        },
    }

    Ok(())
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
        .subcommand(SubCommand::with_name("get")
            .about("Get the value of the counter")
            .version("0.1")
            )
        .subcommand(SubCommand::with_name("next")
            .about("Increment and get the value of the counter")
            .version("0.1")
            )
}

pub fn print_help() -> Result<()> {
    clap_app().print_help()?;

    Ok(())
}

pub fn parse_args<'a>() -> ArgMatches<'a> {
    clap_app().get_matches()
}