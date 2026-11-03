using System;

namespace CodexUsageTray
{
    internal sealed class LimitWindow
    {
        public double UsedPercent;
        public int? WindowMinutes;
        public int? ResetAfterSeconds;
        public string DeviceId;
        public string DisplayName;
        public int? BatteryPercent;
        public string DetailLeft;
        public string DetailRight;
        public string BatteryEstimate;
        public bool IsCached;

        public int RemainingPercent
        {
            get
            {
                double remaining = BatteryPercent.HasValue
                    ? BatteryPercent.Value
                    : 100.0 - UsedPercent;
                if (remaining < 0)
                {
                    remaining = 0;
                }
                if (remaining > 100)
                {
                    remaining = 100;
                }
                return (int)Math.Round(remaining);
            }
        }

        public LimitWindow Clone()
        {
            return (LimitWindow)MemberwiseClone();
        }
    }
}
