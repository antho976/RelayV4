import { migrate } from 'drizzle-orm/expo-sqlite/migrator'
import { useEffect, useState } from 'react'

import { Logger } from '@lib/state/Logger'

import { db, sqliteDB } from './db'

type MigrationConfig = Parameters<typeof migrate>[1]

const MIGRATIONS_TABLE = '__drizzle_migrations'

/** Statements that only add something, and so can be skipped when it is already there. */
const ADDITIVE = /^\s*(CREATE\s+TABLE|CREATE\s+(UNIQUE\s+)?INDEX|ALTER\s+TABLE\s+\S+\s+ADD\b)/i
const ALREADY_THERE = /already exists|duplicate column name/i

/** The SQLite message under drizzle's "Failed to run the query '…'" wrapper. */
export const migrationErrorText = (error: Error): string => {
    const cause = (error as Error & { cause?: unknown }).cause
    const detail = cause instanceof Error ? cause.message : cause ? String(cause) : ''
    return detail && !error.message.includes(detail)
        ? `${error.message}\n\n${detail}`
        : error.message
}

/**
 * A database whose schema is ahead of its `__drizzle_migrations` record — a migration that
 * landed without its row, a database restored from a backup of a different build, an app
 * data folder shared with an older ChatterUI install — fails forever on the first
 * `CREATE TABLE` of the next migration, because the table is already there.
 *
 * This replays every pending migration one statement at a time inside one transaction,
 * skipping a statement only when it adds a table, index or column that already exists, and
 * records each migration as drizzle would. Anything else that fails rolls the whole thing
 * back and rethrows, so a database is never left half-repaired.
 */
const repairPendingMigrations = (config: MigrationConfig) => {
    const { journal, migrations } = config as {
        journal: { entries: { idx: number; when: number; tag: string }[] }
        migrations: Record<string, string>
    }
    sqliteDB.execSync(
        `CREATE TABLE IF NOT EXISTS "${MIGRATIONS_TABLE}" (id SERIAL PRIMARY KEY, hash text NOT NULL, created_at numeric)`
    )
    const last = sqliteDB.getFirstSync<{ created_at: number }>(
        `SELECT created_at FROM "${MIGRATIONS_TABLE}" ORDER BY created_at DESC LIMIT 1`
    )
    const lastApplied = Number(last?.created_at ?? 0)
    const pending = journal.entries.filter((entry) => entry.when > lastApplied)
    let skipped = 0
    sqliteDB.withTransactionSync(() => {
        for (const entry of pending) {
            const source = migrations[`m${entry.idx.toString().padStart(4, '0')}`]
            if (!source) throw new Error(`Missing migration: ${entry.tag}`)
            for (const statement of source.split('--> statement-breakpoint')) {
                if (!statement.trim()) continue
                try {
                    sqliteDB.execSync(statement)
                } catch (e) {
                    const message = `${(e as Error)?.message ?? e}`
                    if (!ADDITIVE.test(statement) || !ALREADY_THERE.test(message)) throw e
                    skipped++
                    Logger.warn(`[MIGRATION] ${entry.tag}: skipped, ${message}`)
                }
            }
            sqliteDB.runSync(
                `INSERT INTO "${MIGRATIONS_TABLE}" ("hash", "created_at") VALUES (?, ?)`,
                '',
                entry.when
            )
        }
    })
    Logger.info(
        `[MIGRATION] repaired ${pending.length} pending migration(s), ${skipped} statement(s) already applied`
    )
}

/**
 * drizzle's `useMigrations`, with one retry through `repairPendingMigrations` when the plain
 * run fails. The first error is the one reported if the repair cannot help either.
 */
export const useMigrations = (config: MigrationConfig) => {
    const [state, setState] = useState<{ success: boolean; error?: Error }>({ success: false })
    useEffect(() => {
        migrate(db, config)
            .then(() => setState({ success: true }))
            .catch((error: Error) => {
                Logger.error(`[MIGRATION] ${migrationErrorText(error)}`)
                try {
                    repairPendingMigrations(config)
                    setState({ success: true })
                } catch (repairError) {
                    Logger.error(`[MIGRATION] repair failed: ${repairError}`)
                    setState({ success: false, error: error })
                }
            })
        // `config` is the bundled migrations module: one run per app start, as drizzle's hook.
    }, [config])
    return state
}
