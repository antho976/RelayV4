import { z } from 'zod'

export const lorebookEntryImportSchema = z
    .object({
        keys: z.array(z.string()),
        content: z.string(),

        // the table and this app's exports say `enable`, the character card spec says `enabled`
        enable: z.boolean().optional(),
        enabled: z.boolean().optional(),
        insertion_order: z.number().int().default(100),
        case_sensitive: z.boolean().default(true),

        name: z.string().default('New Entry'),
        priority: z.number().int().default(100),

        selective: z.boolean().default(false),
        constant: z.boolean().default(false),
        comment: z.string().default(''),
        secondary_keys: z.array(z.string()).default([]),
    })
    .transform(({ enable, enabled, ...entry }) => ({ ...entry, enable: enable ?? enabled ?? true }))

export const lorebookImportSchema = z.object({
    name: z.string().default('New Lorebook'),
    description: z.string().default(''),

    // null is what a lorebook made in the app exports: no limit of its own
    scan_depth: z.number().int().nullable().default(1),
    token_budget: z.number().int().nullable().default(1024),
    recursive_scanning: z.boolean().nullable().default(false),

    entries: z.preprocess((val) => {
        if (typeof val === 'object' && val !== null && !Array.isArray(val)) {
            return Object.values(val)
        }
        return val
    }, z.array(lorebookEntryImportSchema)),
})

export type LorebookImport = z.infer<typeof lorebookImportSchema>
export type LorebookEntryImport = z.infer<typeof lorebookEntryImportSchema>
