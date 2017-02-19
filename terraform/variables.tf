variable "aws_account" {
    description = "aws account number"
}

variable "pgp_key" {
    description = "either a base64 encoded pgp public key or a keybase username in the form keybase:username"
}

variable "monotone_table" {
    description = "dynamodb table name"
    default = "Counters"
}

variable "monotone_table_region" {
    description = "dynamodb table region"
    default = "eu-west-1"
}