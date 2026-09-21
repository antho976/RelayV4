CREATE TABLE `chat_preset_links` (
	`preset_id` integer NOT NULL,
	`chat_id` integer NOT NULL,
	PRIMARY KEY(`preset_id`, `chat_id`),
	FOREIGN KEY (`preset_id`) REFERENCES `chat_presets`(`id`) ON UPDATE no action ON DELETE cascade,
	FOREIGN KEY (`chat_id`) REFERENCES `chats`(`id`) ON UPDATE no action ON DELETE cascade
);
--> statement-breakpoint
CREATE TABLE `chat_presets` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`owner_chat_id` integer NOT NULL,
	`name` text DEFAULT 'New Preset' NOT NULL,
	`system_prompt` text DEFAULT '' NOT NULL,
	`persona` text DEFAULT '' NOT NULL,
	`rules` text DEFAULT '' NOT NULL,
	`create_date` integer NOT NULL,
	FOREIGN KEY (`owner_chat_id`) REFERENCES `chats`(`id`) ON UPDATE no action ON DELETE cascade
);
--> statement-breakpoint
ALTER TABLE `chats` ADD `hidden` integer DEFAULT false NOT NULL;--> statement-breakpoint
ALTER TABLE `chats` ADD `ghost` integer DEFAULT false NOT NULL;--> statement-breakpoint
ALTER TABLE `chats` ADD `memory` text DEFAULT '' NOT NULL;--> statement-breakpoint
ALTER TABLE `chats` ADD `background_image` integer;--> statement-breakpoint
ALTER TABLE `chats` ADD `active_preset_id` integer;--> statement-breakpoint
ALTER TABLE `instructs` ADD `use_card_system_prompt` integer DEFAULT true NOT NULL;--> statement-breakpoint
ALTER TABLE `instructs` ADD `use_post_history` integer DEFAULT true NOT NULL;--> statement-breakpoint
ALTER TABLE `instructs` ADD `label_sections` integer DEFAULT true NOT NULL;--> statement-breakpoint
ALTER TABLE `instructs` ADD `note_in_user_message` integer DEFAULT false NOT NULL;--> statement-breakpoint
ALTER TABLE `instructs` ADD `attachment_depth` integer DEFAULT 2 NOT NULL;--> statement-breakpoint
ALTER TABLE `instructs` ADD `strict_alternation` integer DEFAULT false NOT NULL;