use std::thread;
use std::time::Duration;
use std::default::Default;
use std::collections::HashMap;
use serde_json;
use rusoto::{DefaultCredentialsProvider, Region, ProvideAwsCredentials, DispatchSignedRequest};
use rusoto::dynamodb::*;
use super::error::*;
use super::*;
use string::*;

pub fn list_tables<P,D>(client: &DynamoDbClient<P,D>) ->Result<Vec<String>>
        where P: ProvideAwsCredentials, D: DispatchSignedRequest {
    let list_tables_input: ListTablesInput = Default::default();

    let output = client.list_tables(&list_tables_input)?;

    if let Some(table_names) = output.table_names {
        Ok(table_names)
    } else {
        Ok(vec![])
    }
}

pub fn wait_for_table<P,D>(client: &DynamoDbClient<P,D>, name: &str) -> Result<TableDescription>
        where P: ProvideAwsCredentials, D: DispatchSignedRequest {

    loop {
        let table_desc = describe_table(client, name)?;

        match table_desc.table_status.as_ref().map(|s| &s[..]) {
            Some("ACTIVE") => {
                info!("table {} state ACTIVE", name);
                return Ok(table_desc);
            },
            Some(_) => {
                info!("table {} state {}", name, table_desc.table_status.unwrap());
            },
            None => {
                info!("table {} no state available", name);
            }
        }

        thread::sleep(Duration::from_secs(1));
    }
}

pub fn create_table_if_needed<P,D>(client: &DynamoDbClient<P,D>, name: &str, read_capacity: i64, write_capacity: i64) -> Result<TableDescription>
        where P: ProvideAwsCredentials, D: DispatchSignedRequest {
    loop {
        match describe_table(client, name).map_err(Error::from) {
            Err(Error(ErrorKind::TableNotFound(_), _)) => {
                info!("table {} not found. creating..", name);
            },
            Err(e) => {
                bail!(e);
            },
            Ok(table) => {
                return Ok(table);
            }
        }

        match create_table(client, name, read_capacity, write_capacity) {
            Err(Error(ErrorKind::TableAlreadyExists(_), _)) => {
                info!("table {} already exists. getting info..", name);
            },
            Err(e) => {
                bail!(e);
            },
            Ok(()) => {
                // pass
            }
        }
    }
}

pub fn describe_table<P,D>(client: &DynamoDbClient<P,D>, name: &str) -> Result<TableDescription>
        where P: ProvideAwsCredentials, D: DispatchSignedRequest {
    
    let describe_table_input = DescribeTableInput {
        table_name: name.to_owned(),
        ..Default::default()
    };

    match client.describe_table(&describe_table_input) {
        Err(DescribeTableError::Unknown(json)) => {
            let maybe_value = serde_json::from_str::<AWSError>(&json);
            
            if let Ok(value) = maybe_value {
                if value.message.starts_with("Requested resource not found: Table:") {
                    bail!(ErrorKind::TableNotFound(s(name)));
                }
            }

            bail!(ErrorKind::DescribeTable(DescribeTableError::Unknown(json)));
        },
        Err(e) => {
            bail!(ErrorKind::DescribeTable(e));
        },
        Ok(table) => {
            if let Some(table_desc) = table.table {
                info!("table created at {:?}", table_desc.creation_date_time);
                Ok(table_desc)
            } else {
                bail!(ErrorKind::NoTableInfo);
            }
        }
    }
}

pub fn create_table<P,D>(client: &DynamoDbClient<P,D>, name: &str, read_capacity: i64, write_capacity: i64) -> Result<()>
        where P: ProvideAwsCredentials, D: DispatchSignedRequest {

    let create_table_input = CreateTableInput {
        table_name: name.to_owned(),
        provisioned_throughput: ProvisionedThroughput {
            read_capacity_units: read_capacity,
            write_capacity_units: write_capacity
        },
        attribute_definitions: vec![
            AttributeDefinition {
                attribute_name: s("ID"),
                attribute_type: s("S")
            }
        ],
        key_schema: vec![
            KeySchemaElement {
                attribute_name: s("ID"),
                key_type: s("HASH")
            }
        ],
        ..Default::default()
    };

    match client.create_table(&create_table_input) {
        Err(CreateTableError::Unknown(json)) => {
            let maybe_value = serde_json::from_str::<AWSError>(&json);
            
            if let Ok(value) = maybe_value {
                if value.message.starts_with("Table already exists:") {
                    bail!(ErrorKind::TableAlreadyExists(s(name)));
                }
            }

            bail!(ErrorKind::CreateTable(CreateTableError::Unknown(json)));
        },
        Err(e) => {
            bail!(ErrorKind::CreateTable(e));
        },
        Ok(table) => {
            if let Some(table_desc) = table.table_description {
                info!("table created at {:?}", table_desc.creation_date_time);
                Ok(())
            } else {
                bail!(ErrorKind::NoTableInfo);
            }
        }
    }
}