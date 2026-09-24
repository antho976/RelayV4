import { z } from 'zod'

const CompletionTimingsSchema = z.object({
    predicted_per_token_ms: z.number(),
    predicted_per_second: z.number().nullable(),
    predicted_ms: z.number(),
    predicted_n: z.number(),

    prompt_per_token_ms: z.number(),
    prompt_per_second: z.number().nullable(),
    prompt_ms: z.number(),
    prompt_n: z.number(),
})

const SwipeSchema = z.object({
    swipe: z.string(),
    id: z.number().optional(),
    entry_id: z.number(),
    send_date: z.coerce.date(),
    gen_started: z.coerce.date(),
    gen_finished: z.coerce.date(),
    // exports from before `active` marked the shown swipe with the message's swipe_id
    active: z.boolean().optional(),
    token_length: z.number().nullable().catch(null),
    reset_length: z.number().nullable().catch(null),
    timings: CompletionTimingsSchema.nullable().catch(null),
})

const AttachmentSchema = z.object({
    id: z.number(),
    type: z.enum(['audio', 'image', 'document']),
    name: z.string(),
    chat_entry_id: z.number(),
    uri: z.string(),
    mime_type: z.string(),
    size: z.number(),
})

const MessageSchema = z
    .object({
        id: z.number().optional(),
        name: z.string(),
        chat_id: z.number(),
        is_user: z.boolean(),
        order: z.number(),
        swipe_id: z.number().catch(0),
        swipes: z.array(SwipeSchema),
        attachments: z.array(AttachmentSchema).catch([]),
    })
    .transform((message) => {
        const marked = message.swipes.some((swipe) => swipe.active !== undefined)
        return {
            ...message,
            swipes: message.swipes.map((swipe, index) => ({
                ...swipe,
                active: marked ? (swipe.active ?? false) : index === message.swipe_id,
            })),
        }
    })

export const ChatImportSchema = z.object({
    id: z.number().optional(),
    // fields added after the first export format default when an older file lacks them
    name: z.string().catch('New Chat'),
    last_modified: z.number().nullable().catch(null),
    character_id: z.number(),
    create_date: z.coerce.date(),
    user_id: z.number().nullable().catch(null),
    scroll_offset: z.number().catch(0),
    hidden: z.boolean().catch(false),
    ghost: z.boolean().catch(false),
    memory: z.string().catch(''),
    background_image: z.number().nullable().catch(null),
    active_preset_id: z.number().nullable().catch(null),
    messages: z.array(MessageSchema),
})

export type CompletionTimings = z.infer<typeof CompletionTimingsSchema>
export type Swipe = z.infer<typeof SwipeSchema>
export type Attachment = z.infer<typeof AttachmentSchema>
export type Message = z.infer<typeof MessageSchema>
export type Chat = z.infer<typeof ChatImportSchema>
