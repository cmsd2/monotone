# Optional real-AWS test infrastructure

CI does not use this module. GitHub Actions runs the integration tests against
DynamoDB Local, with no AWS account or secrets. See the main README.

Use this module when you want to run the same tests against a real DynamoDB
table. It creates:

- an IAM user `monotone_test` with an access key, the secret encrypted with your PGP key
- an inline policy allowing `CreateTable`, `DescribeTable`, `GetItem`,
  `PutItem` and `DeleteItem` on one table, plus `ListTables`

The tests and the CLI create the table on first use, so the module does not
manage the table itself.

## Apply

```sh
cd terraform
terraform init
terraform apply \
  -var aws_account=123456789012 \
  -var pgp_key=keybase:yourname
```

Put the variables in a `*.tfvars` file if you prefer; those files are gitignored.
Configure a remote state backend of your choice before sharing state.

## Use the credentials

```sh
export AWS_ACCESS_KEY_ID=$(terraform output -raw test_user_access_key_id)
export AWS_SECRET_ACCESS_KEY=$(terraform output -raw test_user_encrypted_secret | base64 --decode | keybase pgp decrypt)
export AWS_REGION=eu-west-1
# The integration tests only run when AWS_ENDPOINT_URL is set.
export AWS_ENDPOINT_URL=https://dynamodb.eu-west-1.amazonaws.com

cd ..
MONOTONE_REQUIRE_INTEGRATION=1 cargo test -p monotone --all-features --test dynamodb -- \
  --skip table_create_describe_and_errors --skip concurrent_table_creation_is_tolerated
```

Apply with `-var monotone_table=monotone-it`, the table the library
integration tests share.

Two library tests and every CLI test create short-lived tables with generated
names, which this policy does not allow. The command above skips the two
library tests. Run the CLI tests against DynamoDB Local, or grant
`CreateTable`, `DescribeTable` and `DeleteTable` on `table/*` to a separate user.

Do not commit the decrypted secret or put it in CI. Destroy the user with
`terraform destroy` when you are done.
