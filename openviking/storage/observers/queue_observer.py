# Copyright (c) 2026 Beijing Volcano Engine Technology Co., Ltd.
# SPDX-License-Identifier: Apache-2.0
"""
QueueObserver: Queue system observability tool.

Provides methods to observe and report queue status in various formats.
"""

import asyncio
from typing import Dict

from openviking.storage.observers.base_observer import BaseObserver
from openviking.storage.queuefs.named_queue import QueueStatus
from openviking.storage.queuefs.queue_manager import QueueManager
from openviking.utils.logger import get_logger

logger = get_logger(__name__)


class QueueObserver(BaseObserver):
    """
    QueueObserver: System observability tool for queue management.

    Provides methods to query queue status and format output.
    """

    def __init__(self, queue_manager: QueueManager):
        self._queue_manager = queue_manager

    def get_status_table(self) -> str:
        statuses = asyncio.run(self._queue_manager.check_status())
        return self._format_status_as_table(statuses)

    def __str__(self) -> str:
        return self.get_status_table()

    def _format_status_as_table(self, statuses: Dict[str, QueueStatus]) -> str:
        """
        Format queue statuses as a pandas DataFrame table.

        Args:
            statuses: Dict mapping queue names to QueueStatus

        Returns:
            Formatted table string
        """
        import pandas as pd

        if not statuses:
            return "No queue status data available."

        data = []
        total_pending = 0
        total_in_progress = 0
        total_processed = 0
        total_errors = 0

        for queue_name, status in statuses.items():
            total = status.pending + status.in_progress + status.processed
            data.append(
                {
                    "Queue": queue_name,
                    "Pending": status.pending,
                    "In Progress": status.in_progress,
                    "Processed": status.processed,
                    "Errors": status.error_count,
                    "Total": total,
                }
            )
            total_pending += status.pending
            total_in_progress += status.in_progress
            total_processed += status.processed
            total_errors += status.error_count

        df = pd.DataFrame(data)

        # Add total row
        total_total = total_pending + total_in_progress + total_processed
        total_row = {
            "Queue": "TOTAL",
            "Pending": total_pending,
            "In Progress": total_in_progress,
            "Processed": total_processed,
            "Errors": total_errors,
            "Total": total_total,
        }
        df = pd.concat([df, pd.DataFrame([total_row])], ignore_index=True)

        return df.to_string(
            index=False,
            col_space={
                "Queue": 20,
                "Pending": 10,
                "In Progress": 12,
                "Processed": 10,
                "Errors": 8,
                "Total": 10,
            },
        )

    def is_healthy(self) -> bool:
        return not self.has_errors()

    def has_errors(self) -> bool:
        return self._queue_manager.has_errors()
