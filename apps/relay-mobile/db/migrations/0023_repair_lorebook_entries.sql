-- 0021 copied "selective", "constant", "comment" and "secondary_keys" from a table that had
-- none of them, so SQLite stored the column names as string literals.
UPDATE `lorebook_entries` SET `comment` = '' WHERE `comment` = 'comment' AND (`selective` = 'selective' OR `secondary_keys` = 'secondary_keys');--> statement-breakpoint
UPDATE `lorebook_entries` SET `secondary_keys` = '[]' WHERE `secondary_keys` IN ('secondary_keys', '');--> statement-breakpoint
UPDATE `lorebook_entries` SET `keys` = '[]' WHERE `keys` = '';--> statement-breakpoint
UPDATE `lorebook_entries` SET `selective` = false WHERE typeof(`selective`) <> 'integer';--> statement-breakpoint
UPDATE `lorebook_entries` SET `constant` = false WHERE typeof(`constant`) <> 'integer';
