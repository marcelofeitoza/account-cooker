CREATE TRIGGER action_events_reject_update
BEFORE UPDATE ON action_events
BEGIN
    SELECT RAISE(ABORT, 'action_events are immutable');
END;

CREATE TRIGGER action_events_reject_delete
BEFORE DELETE ON action_events
BEGIN
    SELECT RAISE(ABORT, 'action_events are immutable');
END;

CREATE TRIGGER traces_reject_update
BEFORE UPDATE ON traces
BEGIN
    SELECT RAISE(ABORT, 'traces are immutable');
END;

CREATE TRIGGER traces_reject_delete
BEFORE DELETE ON traces
BEGIN
    SELECT RAISE(ABORT, 'traces are immutable');
END;

CREATE TRIGGER prepared_transactions_reject_update
BEFORE UPDATE ON prepared_transactions
BEGIN
    SELECT RAISE(ABORT, 'prepared transactions are immutable');
END;

CREATE TRIGGER prepared_transactions_reject_delete
BEFORE DELETE ON prepared_transactions
BEGIN
    SELECT RAISE(ABORT, 'prepared transactions are immutable');
END;

CREATE TRIGGER submissions_reject_update
BEFORE UPDATE ON submissions
BEGIN
    SELECT RAISE(ABORT, 'submissions are immutable');
END;

CREATE TRIGGER submissions_reject_delete
BEFORE DELETE ON submissions
BEGIN
    SELECT RAISE(ABORT, 'submissions are immutable');
END;

CREATE TRIGGER simulations_reject_update
BEFORE UPDATE ON simulations
BEGIN
    SELECT RAISE(ABORT, 'simulations are immutable');
END;

CREATE TRIGGER simulations_reject_delete
BEFORE DELETE ON simulations
BEGIN
    SELECT RAISE(ABORT, 'simulations are immutable');
END;

CREATE TRIGGER receipts_reject_update
BEFORE UPDATE ON receipts
BEGIN
    SELECT RAISE(ABORT, 'receipts are immutable');
END;

CREATE TRIGGER receipts_reject_delete
BEFORE DELETE ON receipts
BEGIN
    SELECT RAISE(ABORT, 'receipts are immutable');
END;
