use rusoto;
use std::num;
use serde_json;

error_chain! {
    foreign_links {
        DescribeTable(rusoto::dynamodb::DescribeTableError);
        ListTables(rusoto::dynamodb::ListTablesError);
        CreateTable(rusoto::dynamodb::CreateTableError);
        GetItem(rusoto::dynamodb::GetItemError);
        PutItem(rusoto::dynamodb::PutItemError);
        ParseError(num::ParseIntError);
        Json(serde_json::Error);
    }

    errors {
        NoTableInfo {
            description("no table info returned")
            display("no table info returned")
        }

        TableAlreadyExists(t: String) {
            description("table already exists")
            display("table already exists: {}", t)
        }

        TableNotFound(t: String) {
            description("table not found")
            display("table not found: {}", t)
        }

        ConditionalUpdateFailed {
            description("conditional update failed")
            display("conditional update failed")
        }

        UnrecognisedCounterType {
            description("unrecognised counter type")
            display("unrecognised counter type")
        }

        MissingAttribute {
            description("missing attribute")
            display("missing attribute")
        }
    }
}