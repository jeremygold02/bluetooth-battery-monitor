using System;
using System.Runtime.InteropServices;
using Windows.Devices.Bluetooth;
using Windows.Devices.Enumeration;

namespace CodexUsageTray
{
    // Watches connections only; battery readings still come from the collectors.
    internal sealed class BluetoothConnectionWatcher : IDisposable
    {
        private readonly object sync = new object();
        private readonly Action onChanged;
        private DeviceWatcher watcher;
        private bool initialized;
        private bool removedDuringEnumeration;
        private bool disposed;

        public BluetoothConnectionWatcher(Action onChanged)
        {
            if (onChanged == null)
            {
                throw new ArgumentNullException("onChanged");
            }
            this.onChanged = onChanged;
        }

        public void Start()
        {
            lock (sync)
            {
                if (disposed)
                {
                    throw new ObjectDisposedException("BluetoothConnectionWatcher");
                }
                if (watcher != null)
                {
                    return;
                }

                string selector = BluetoothLEDevice.GetDeviceSelectorFromConnectionStatus(
                    BluetoothConnectionStatus.Connected);
                DeviceWatcher next = DeviceInformation.CreateWatcher(selector);
                watcher = next;
                try
                {
                    next.Added += WatcherAdded;
                    next.Removed += WatcherRemoved;
                    next.EnumerationCompleted += WatcherEnumerationCompleted;
                    next.Start();
                }
                catch
                {
                    Unregister(next);
                    watcher = null;
                    throw;
                }
            }
        }

        private void WatcherAdded(DeviceWatcher sender, DeviceInformation value)
        {
            NotifyIfReady(sender, false);
        }

        private void WatcherRemoved(DeviceWatcher sender, DeviceInformationUpdate value)
        {
            NotifyIfReady(sender, true);
        }

        private void NotifyIfReady(DeviceWatcher sender, bool removed)
        {
            bool notify;
            lock (sync)
            {
                if (disposed || sender != watcher)
                {
                    return;
                }
                if (!initialized && removed)
                {
                    removedDuringEnumeration = true;
                }
                notify = initialized;
            }
            if (notify)
            {
                onChanged();
            }
        }

        private void WatcherEnumerationCompleted(DeviceWatcher sender, object value)
        {
            bool notify;
            lock (sync)
            {
                if (disposed || sender != watcher)
                {
                    return;
                }
                initialized = true;
                notify = removedDuringEnumeration;
                removedDuringEnumeration = false;
            }
            if (notify)
            {
                onChanged();
            }
        }

        public void Dispose()
        {
            DeviceWatcher previous;
            lock (sync)
            {
                if (disposed)
                {
                    return;
                }
                disposed = true;
                previous = watcher;
                watcher = null;
            }
            if (previous == null)
            {
                return;
            }

            Unregister(previous);
            DeviceWatcherStatus status = previous.Status;
            if (status == DeviceWatcherStatus.Started ||
                status == DeviceWatcherStatus.EnumerationCompleted)
            {
                try
                {
                    previous.Stop();
                }
                catch (InvalidOperationException)
                {
                    // The watcher can stop between checking its status and stopping it.
                }
                catch (COMException)
                {
                    // Windows may have already stopped the watcher.
                }
            }
        }

        private void Unregister(DeviceWatcher value)
        {
            value.Added -= WatcherAdded;
            value.Removed -= WatcherRemoved;
            value.EnumerationCompleted -= WatcherEnumerationCompleted;
        }
    }
}
