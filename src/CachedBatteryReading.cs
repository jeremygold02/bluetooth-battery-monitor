using System;

namespace CodexUsageTray
{
    internal sealed class CachedBatteryReading
    {
        public string Id { get; set; }
        public string PhysicalId { get; set; }
        public string Name { get; set; }
        public string Source { get; set; }
        public int Percent { get; set; }
        public int Priority { get; set; }
        public DateTime ObservedAt { get; set; }

        internal CachedBatteryReading Clone()
        {
            return (CachedBatteryReading)MemberwiseClone();
        }
    }
}
