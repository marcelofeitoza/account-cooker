ALTER TABLE actions
ADD COLUMN max_account_creation_lamports INTEGER NOT NULL DEFAULT 0
CHECK (max_account_creation_lamports >= 0);
