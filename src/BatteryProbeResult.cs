using System;

namespace CodexUsageTray
{
    internal sealed class BatteryProbeResult
    {
        public string Id;
        public string PhysicalId;
        public string Name;
        public int? Percent;
        public string Source;
        public string Status;
        public int Priority;
        public DateTime ObservedAt;
        public bool IsCached;
        public string BatteryEstimate;

        public BatteryProbeResult Clone()
        {
            return (BatteryProbeResult)MemberwiseClone();
        }
    }
}
