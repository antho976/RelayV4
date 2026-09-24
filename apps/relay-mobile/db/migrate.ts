import { is, SQL } from 'drizzle-orm'
import { migrate } from 'drizzle-orm/expo-sqlite/migrator'
import { getTableConfig, SQLiteTable } from 'drizzle-orm/sqlite-core'
import { useEffect, useState } from 'react'

import { Logger } from '@lib/state/Logger'

import { db, sqliteDB } from './db'
import * as schema from './schema'

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
 * data folder shared with an older install — fails forever on the first
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

/** A literal SQL default for a column, or undefined when it has none a migration could use. */
const sqlDefault = (value: unknown): string | undefined => {
    if (value === undefined || value === null || is(value, SQL)) return undefined
    if (typeof value === 'boolean') return value ? '1' : '0'
    if (typeof value === 'number') return String(value)
    if (typeof value === 'string') return `'${value.replace(/'/g, "''")}'`
    return undefined
}

/**
 * Add any column the app's schema has and the database lacks. A database whose tables came
 * from another build (a restored backup, an older install) can pass every
 * migration and still miss a column; every query that names it then fails, and a list that
 * reads it — the character list — comes back empty with no error on screen.
 *
 * Only additions: nothing is dropped, renamed or rewritten. A column's default, or the value
 * its `$defaultFn` makes as stored, fills existing rows; a NOT NULL column with neither gets
 * the type's zero (`'[]'` for JSON), which is what SQLite needs to add it to existing rows.
 */
export const reconcileSchema = () => {
    let added = 0
    for (const table of Object.values(schema)) {
        if (!is(table, SQLiteTable)) continue
        const config = getTableConfig(table)
        const present = sqliteDB.getAllSync<{ name: string }>(`PRAGMA table_info("${config.name}")`)
        // A missing table is the migrations' job; creating it by hand here could not match.
        if (present.length === 0) continue
        const names = new Set(present.map((column) => column.name))
        for (const column of config.columns) {
            if (names.has(column.name) || column.primary) continue
            const type = column.getSQLType()
            const initial = column.default ?? column.defaultFn?.()
            let fallback = sqlDefault(
                initial === undefined || is(initial, SQL)
                    ? initial
                    : column.mapToDriverValue(initial)
            )
            if (fallback === undefined && column.notNull)
                fallback = /int|real|num/i.test(type)
                    ? '0'
                    : column.columnType === 'SQLiteTextJson'
                      ? "'[]'"
                      : "''"
            const statement =
                `ALTER TABLE "${config.name}" ADD "${column.name}" ${type}` +
                (fallback !== undefined ? ` DEFAULT ${fallback}` : '') +
                (column.notNull && fallback !== undefined ? ' NOT NULL' : '')
            try {
                sqliteDB.execSync(statement)
                added++
                Logger.warn(`[MIGRATION] added missing column ${config.name}.${column.name}`)
            } catch (e) {
                Logger.error(`[MIGRATION] could not add ${config.name}.${column.name}: ${e}`)
            }
        }
    }
    if (added > 0) Logger.info(`[MIGRATION] schema reconciled: ${added} column(s) added`)
}

/**
 * drizzle's `useMigrations`, with one retry through `repairPendingMigrations` when the plain
 * run fails. The first error is the one reported if the repair cannot help either.
 */
export const useMigrations = (config: MigrationConfig) => {
    const [state, setState] = useState<{ success: boolean; error?: Error }>({ success: false })
    useEffect(() => {
        migrate(db, config)
            .then(() => {
                try {
                    reconcileSchema()
                } catch (e) {
                    Logger.error(`[MIGRATION] schema check failed: ${e}`)
                }
                setState({ success: true })
            })
            .catch((error: Error) => {
                Logger.error(`[MIGRATION] ${migrationErrorText(error)}`)
                try {
                    repairPendingMigrations(config)
                    reconcileSchema()
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
