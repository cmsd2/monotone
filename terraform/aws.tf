provider "aws" {
    allowed_account_ids = ["${var.aws_account}"]
}