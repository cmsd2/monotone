variable "aws_account" {
  description = "AWS account ID the resources are created in"
  type        = string
}

variable "pgp_key" {
  description = "Base64-encoded PGP public key, or keybase:username, used to encrypt the test user's secret access key"
  type        = string
}

variable "monotone_table" {
  description = "DynamoDB table the tests use"
  type        = string
  default     = "Counters"
}

variable "monotone_table_region" {
  description = "Region of the DynamoDB table"
  type        = string
  default     = "eu-west-1"
}
