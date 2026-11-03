using System.Collections.Generic;

namespace CodexUsageTray
{
    // Samples represent qualified, non-overlapping observations.
    internal sealed class LearnedBatteryRateSample
    {
        public double Percent { get; set; }
        public double Seconds { get; set; }

        public LearnedBatteryRateSample() { }

        public LearnedBatteryRateSample Clone()
        {
            return (LearnedBatteryRateSample)MemberwiseClone();
        }
    }

    internal sealed class LearnedBatteryRate
    {
        public string DeviceId { get; set; }
        public List<LearnedBatteryRateSample> ChargeSamples { get; set; }
        public List<LearnedBatteryRateSample> DischargeSamples { get; set; }

        public LearnedBatteryRate()
        {
            ChargeSamples = new List<LearnedBatteryRateSample>();
            DischargeSamples = new List<LearnedBatteryRateSample>();
        }

        public LearnedBatteryRate Clone()
        {
            LearnedBatteryRate copy = new LearnedBatteryRate { DeviceId = DeviceId };
            CopySamples(ChargeSamples, copy.ChargeSamples);
            CopySamples(DischargeSamples, copy.DischargeSamples);
            return copy;
        }

        private static void CopySamples(List<LearnedBatteryRateSample> source, List<LearnedBatteryRateSample> destination)
        {
            if (source == null) return;
            foreach (LearnedBatteryRateSample sample in source)
                if (sample != null) destination.Add(sample.Clone());
        }
    }
}
