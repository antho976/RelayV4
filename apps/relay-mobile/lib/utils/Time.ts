export const getFriendlyTimeStamp = (oldtime: number) => {
    const midnight = new Date()
    midnight.setHours(0, 0, 0, 0)
    if (oldtime >= midnight.getTime()) return new Date(oldtime).toLocaleTimeString()
    midnight.setDate(midnight.getDate() - 1)
    if (oldtime >= midnight.getTime()) return 'Yesterday'
    return new Date(oldtime).toLocaleDateString()
}
