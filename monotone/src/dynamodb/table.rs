//! Table provisioning.

use std::time::Duration;

use aws_sdk_dynamodb::Client;
use aws_sdk_dynamodb::types::{
    AttributeDefinition, KeySchemaElement, KeyType, ProvisionedThroughput, ScalarAttributeType,
    TableDescription, TableStatus,
};
use tracing::info;

use super::error::Error;
use crate::core::encode::ID;

/// Lists every table name visible to the client.
pub async fn list_tables(client: &Client) -> Result<Vec<String>, Error> {
    let mut names = Vec::new();
    let mut start: Option<String> = None;
    loop {
        let out = client
            .list_tables()
            .set_exclusive_start_table_name(start.take())
            .send()
            .await
            .map_err(|e| Error::sdk("ListTables", e))?;
        names.extend_from_slice(out.table_names());
        match out.last_evaluated_table_name() {
            Some(last) => start = Some(last.to_owned()),
            None => return Ok(names),
        }
    }
}

/// Describes a table. A missing table is [`Error::TableNotFound`].
pub async fn describe_table(client: &Client, name: &str) -> Result<TableDescription, Error> {
    match client.describe_table().table_name(name).send().await {
        Ok(out) => out.table.ok_or_else(|| Error::NoTableInfo(name.to_owned())),
        Err(e)
            if e.as_service_error()
                .is_some_and(|s| s.is_resource_not_found_exception()) =>
        {
            Err(Error::TableNotFound(name.to_owned()))
        }
        Err(e) => Err(Error::sdk("DescribeTable", e)),
    }
}

/// Creates a table keyed by string `ID` with the given provisioned throughput.
/// An existing table is [`Error::TableAlreadyExists`].
pub async fn create_table(
    client: &Client,
    name: &str,
    read_capacity: i64,
    write_capacity: i64,
) -> Result<(), Error> {
    let result = client
        .create_table()
        .table_name(name)
        .attribute_definitions(
            AttributeDefinition::builder()
                .attribute_name(ID)
                .attribute_type(ScalarAttributeType::S)
                .build()?,
        )
        .key_schema(
            KeySchemaElement::builder()
                .attribute_name(ID)
                .key_type(KeyType::Hash)
                .build()?,
        )
        .provisioned_throughput(
            ProvisionedThroughput::builder()
                .read_capacity_units(read_capacity)
                .write_capacity_units(write_capacity)
                .build()?,
        )
        .send()
        .await;

    match result {
        Ok(_) => {
            info!(table = name, "created table");
            Ok(())
        }
        Err(e)
            if e.as_service_error()
                .is_some_and(|s| s.is_resource_in_use_exception()) =>
        {
            Err(Error::TableAlreadyExists(name.to_owned()))
        }
        Err(e) => Err(Error::sdk("CreateTable", e)),
    }
}

/// Returns the table's description, creating the table first if it is missing.
/// A concurrent creation by another process is tolerated.
pub async fn create_table_if_needed(
    client: &Client,
    name: &str,
    read_capacity: i64,
    write_capacity: i64,
) -> Result<TableDescription, Error> {
    loop {
        match describe_table(client, name).await {
            Ok(table) => return Ok(table),
            Err(Error::TableNotFound(_)) => info!(table = name, "table not found, creating"),
            Err(e) => return Err(e),
        }
        match create_table(client, name, read_capacity, write_capacity).await {
            Ok(()) => {}
            Err(Error::TableAlreadyExists(_)) => {
                info!(table = name, "table already exists, describing")
            }
            Err(e) => return Err(e),
        }
    }
}

/// Polls `DescribeTable` once per second until the table is `ACTIVE`.
pub async fn wait_for_table(client: &Client, name: &str) -> Result<TableDescription, Error> {
    loop {
        let table = describe_table(client, name).await?;
        match table.table_status() {
            Some(TableStatus::Active) => return Ok(table),
            Some(status) => info!(table = name, status = status.as_str(), "waiting for table"),
            None => info!(table = name, "waiting for table, no status yet"),
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
