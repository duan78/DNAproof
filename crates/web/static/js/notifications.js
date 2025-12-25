/**
 * Modern Notification System
 * Provides toast-style notifications for better UX than browser alerts
 */

class NotificationSystem {
    constructor() {
        this.container = null;
        this.notifications = [];
        this.init();
    }

    init() {
        // Create the container if it doesn't exist
        if (!document.getElementById('notification-container')) {
            this.container = document.createElement('div');
            this.container.id = 'notification-container';
            document.body.appendChild(this.container);
        } else {
            this.container = document.getElementById('notification-container');
        }
    }

    /**
     * Show a notification
     * @param {string} type - Type: 'success', 'error', 'warning', 'info'
     * @param {string} title - Title text
     * @param {string} message - Message text (optional)
     * @param {number} duration - Duration in ms (0 for no auto-dismiss)
     */
    show(type, title, message = '', duration = 5000) {
        const notification = this.createNotification(type, title, message);
        this.container.appendChild(notification);
        this.notifications.push(notification);

        // Auto-dismiss after duration
        if (duration > 0) {
            setTimeout(() => {
                this.dismiss(notification);
            }, duration);
        }

        return notification;
    }

    /**
     * Convenience methods for different types
     */
    success(title, message = '', duration = 5000) {
        return this.show('success', title, message, duration);
    }

    error(title, message = '', duration = 0) {
        // Errors don't auto-dismiss by default
        return this.show('error', title, message, duration);
    }

    warning(title, message = '', duration = 5000) {
        return this.show('warning', title, message, duration);
    }

    info(title, message = '', duration = 5000) {
        return this.show('info', title, message, duration);
    }

    /**
     * Create a notification element
     */
    createNotification(type, title, message) {
        const notification = document.createElement('div');
        notification.className = `notification ${type}`;

        const icons = {
            success: '✓',
            error: '✕',
            warning: '⚠',
            info: 'ℹ'
        };

        notification.innerHTML = `
            <div class="notification-icon">${icons[type] || icons.info}</div>
            <div class="notification-content">
                <div class="notification-title">${this.escapeHtml(title)}</div>
                ${message ? `<div class="notification-message">${this.escapeHtml(message)}</div>` : ''}
            </div>
            <button class="notification-close" aria-label="Close notification">×</button>
        `;

        // Add click handler for close button
        const closeBtn = notification.querySelector('.notification-close');
        closeBtn.addEventListener('click', () => this.dismiss(notification));

        return notification;
    }

    /**
     * Dismiss a notification
     */
    dismiss(notification) {
        if (!notification || !notification.parentElement) return;

        notification.classList.add('removing');
        notification.addEventListener('animationend', () => {
            if (notification.parentElement) {
                notification.remove();
            }
            this.notifications = this.notifications.filter(n => n !== notification);
        });
    }

    /**
     * Dismiss all notifications
     */
    dismissAll() {
        this.notifications.forEach(notification => {
            this.dismiss(notification);
        });
    }

    /**
     * Escape HTML to prevent XSS
     */
    escapeHtml(text) {
        const div = document.createElement('div');
        div.textContent = text;
        return div.innerHTML;
    }
}

// Create global instance
const notificationSystem = new NotificationSystem();

// Make it available globally
window.notificationSystem = notificationSystem;

// Export for module usage
if (typeof module !== 'undefined' && module.exports) {
    module.exports = NotificationSystem;
}
