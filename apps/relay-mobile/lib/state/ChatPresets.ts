import { and, desc, eq, inArray, notInArray, or } from 'drizzle-orm'

import { db as database } from '@db/db'
import { chatPresetLinks, chatPresets, ChatPresetType, chats } from '@db/schema'

import { Logger } from './Logger'

/**
 * Chat presets bundle a system prompt, a persona (user description) and rules.
 *
 * Ownership model:
 * - A preset is created inside a chat, which becomes its owner.
 * - A preset is visible in its owner chat and in every chat it was given to.
 * - Only the owner chat can edit a preset, delete it, or give it to other chats.
 * - A chat that received a preset can only activate it or remove it from itself.
 */
export namespace ChatPresets {
    export type ChatPresetEdit = Pick<
        ChatPresetType,
        'name' | 'system_prompt' | 'persona' | 'rules'
    >

    export const blankPreset = (): ChatPresetEdit => ({
        name: 'New Preset',
        system_prompt: '',
        persona: '',
        rules: '',
    })

    /** true when the preset contributes anything to the prompt */
    export const isEmpty = (preset?: ChatPresetType | null) =>
        !preset ||
        (preset.system_prompt.trim() === '' &&
            preset.persona.trim() === '' &&
            preset.rules.trim() === '')

    export namespace db {
        export namespace query {
            /**
             * Presets available to a chat: the ones it owns plus the ones it was given.
             */
            export const presetsForChatQuery = (chatId: number) => {
                const given = database
                    .select({ id: chatPresetLinks.preset_id })
                    .from(chatPresetLinks)
                    .where(eq(chatPresetLinks.chat_id, chatId))
                return database
                    .select()
                    .from(chatPresets)
                    .where(
                        or(eq(chatPresets.owner_chat_id, chatId), inArray(chatPresets.id, given))
                    )
                    .orderBy(desc(chatPresets.create_date))
            }

            export const preset = async (presetId: number) => {
                return await database.query.chatPresets.findFirst({
                    where: eq(chatPresets.id, presetId),
                })
            }

            /**
             * The active preset of a chat, verified to still be available to that chat.
             */
            export const activeForChat = async (
                chatId: number,
                presetId?: number | null
            ): Promise<ChatPresetType | undefined> => {
                if (!presetId) return
                const result = await database.query.chatPresets.findFirst({
                    where: eq(chatPresets.id, presetId),
                    with: { links: true },
                })
                if (!result) return
                const available =
                    result.owner_chat_id === chatId ||
                    result.links.some((item) => item.chat_id === chatId)
                if (!available) {
                    Logger.warn(`Preset ${presetId} is no longer available to chat ${chatId}`)
                    return
                }
                const { links, ...preset } = result
                return preset
            }

            /** chats of the same character that a preset can still be given to */
            export const giveTargets = async (presetId: number, ownerChatId: number) => {
                const owner = await database.query.chats.findFirst({
                    where: eq(chats.id, ownerChatId),
                    columns: { character_id: true },
                })
                if (!owner) return []
                const links = await database
                    .select({ id: chatPresetLinks.chat_id })
                    .from(chatPresetLinks)
                    .where(eq(chatPresetLinks.preset_id, presetId))
                const excluded = [ownerChatId, ...links.map((item) => item.id)]
                return await database
                    .select({ id: chats.id, name: chats.name, last_modified: chats.last_modified })
                    .from(chats)
                    .where(
                        and(
                            eq(chats.character_id, owner.character_id),
                            eq(chats.ghost, false),
                            notInArray(chats.id, excluded)
                        )
                    )
                    .orderBy(desc(chats.last_modified))
            }

            export const linkedChats = async (presetId: number) => {
                return await database
                    .select({ id: chats.id, name: chats.name })
                    .from(chatPresetLinks)
                    .innerJoin(chats, eq(chatPresetLinks.chat_id, chats.id))
                    .where(eq(chatPresetLinks.preset_id, presetId))
            }
        }

        export namespace mutate {
            export const createPreset = async (ownerChatId: number, data: ChatPresetEdit) => {
                const [{ id }] = await database
                    .insert(chatPresets)
                    .values({ ...data, owner_chat_id: ownerChatId })
                    .returning({ id: chatPresets.id })
                return id
            }

            export const updatePreset = async (presetId: number, data: ChatPresetEdit) => {
                await database.update(chatPresets).set(data).where(eq(chatPresets.id, presetId))
            }

            /**
             * Deletes a preset and clears it from every chat that had it active.
             */
            export const deletePreset = async (presetId: number) => {
                await database
                    .update(chats)
                    .set({ active_preset_id: null })
                    .where(eq(chats.active_preset_id, presetId))
                await database.delete(chatPresets).where(eq(chatPresets.id, presetId))
            }

            export const giveToChat = async (presetId: number, chatId: number) => {
                await database
                    .insert(chatPresetLinks)
                    .values({ preset_id: presetId, chat_id: chatId })
                    .onConflictDoNothing()
            }

            /**
             * Removes a given preset from a chat, deactivating it there if needed.
             */
            export const removeFromChat = async (presetId: number, chatId: number) => {
                await database
                    .delete(chatPresetLinks)
                    .where(
                        and(
                            eq(chatPresetLinks.preset_id, presetId),
                            eq(chatPresetLinks.chat_id, chatId)
                        )
                    )
                await database
                    .update(chats)
                    .set({ active_preset_id: null })
                    .where(and(eq(chats.id, chatId), eq(chats.active_preset_id, presetId)))
            }
        }
    }
}
