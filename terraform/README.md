# Monotone Test Infra using Terraform

Suggested environment variables:

```
AWS_ACCESS_KEY=...
AWS_SECRET_ACCESS_KEY=...
AWS_DEFAULT_REGION=...
```

Example s3 terraform remote config:

```
terraform remote config \
  -backend=s3 \
  -backend-config="bucket=cmsd2-config" \
  -backend-config="key=monotone/terraform.tfstate" \
  -backend-config="region=eu-west-1"
```

Then run `terraform plan` and `terraform apply` as normal.

Extracting the secret access key (using keybase):

```
terraform ouput travis_user_secret | base64 --decode | keybase pgp decrypt
```

Finally use travis's encrypted environment variables:

```
cd path-to-monotone-root
travis encrypt AWS_SECRET_ACCESS_KEY=super_secret --add env.global
```