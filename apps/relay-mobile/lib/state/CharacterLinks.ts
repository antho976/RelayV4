import { and, eq, gt, sql } from 'drizzle-orm'

import { db as database } from '@db/db'
import { characterLinks, LinkType } from '@db/schema'

export namespace CharacterLink {
    export namespace db {
        export namespace mutate {
            export const upsert = async (character_id: number, type: LinkType, value: number) => {
                await database
                    .insert(characterLinks)
                    .values({ character_id, type, value })
                    .onConflictDoUpdate({
                        target: [characterLinks.character_id, characterLinks.type],
                        set: { value },
                    })
            }

            export const deleteById = async (id: number) => {
                await database.delete(characterLinks).where(eq(characterLinks.id, id))
            }

            export const deleteByValue = async (type: LinkType, value: number) => {
                // sync on purpose: expo-sqlite commits an async transaction at its first await
                database.transaction((tx) => {
                    tx.delete(characterLinks)
                        .where(and(eq(characterLinks.value, value), eq(characterLinks.type, type)))
                        .run()
                    // for _index stores, need to shift all values greater than [value] by -1 on removals
                    if (type.endsWith('index')) {
                        tx.update(characterLinks)
                            .set({
                                value: sql`${characterLinks.value} - 1`,
                            })
                            .where(
                                and(eq(characterLinks.type, type), gt(characterLinks.value, value))
                            )
                            .run()
                    }
                })
            }
        }

        export namespace live {
            export const links = (character_id: number) => {
                return database.query.characterLinks.findMany({
                    where: eq(characterLinks.character_id, character_id),
                })
            }
        }
    }
}
